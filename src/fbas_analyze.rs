use crate::fbas::{Fbas, FbasError};
use batsat::{
    interface::SolveResult, intmap::AsIndex, lbool, theory, Callbacks, Lit, Solver,
    SolverInterface, Var,
};
use itertools::Itertools;
use petgraph::{csr::IndexType, graph::NodeIndex};

// Two imaginary quorums A and B, and we have FBAS system with V vertices. Note
// the a quorum contain validators, whereas a vertex can be either a validator
// or a qset. The relation of each vertex being in each quorum is represented
// with one atomic variable (also known as a literal or `lit`). A `true` value
// indicates the validator is in the quorum. Therefore the entire system has `2
// * length(V)` such native atomics. Then we build a set of constrains that must
// be satisfied in order to fail the validity condition (there only needs to be
// one such instance to disprove a condition), We hand the constrain to the
// solver, which tries to find a configuration in atomics such that this
// constrain satisfies, which is sufficient to disprove quorum intersection
// property.
//
// Constrain #1: A is not empty and B is not empty
// Constrain #2: There do NOT exist a validator in both quorums
// Constrain #3: If a vertex is in a quorum, then the quorum must satisfy its
// dependencies. I.e. any threshold number out of all successors needs to be in
// the quorum as well.
//
// These three constrains must all be hold.
//
// Before we send these constrains to a SAT solver, they must be transformed
// into conjunction norm form (CNF). Thus a portion of work is to perform the
// Tseitin transformation on the constrains into "AND of ORs", which also
// introduces additional imaginary atomics. Once the solver produces a
// satisfiable result (result == SAT), that means a disjoint quorum has been
// found.

struct FbasLitsWrapper {
    vertex_count: usize,
}

impl FbasLitsWrapper {
    fn new(vcount: usize) -> Self {
        Self {
            vertex_count: vcount,
        }
    }

    fn in_quorum_a(&self, ni: &NodeIndex) -> Lit {
        Lit::new(Var::from_index(ni.index()), true)
    }

    fn in_quorum_b(&self, ni: &NodeIndex) -> Lit {
        Lit::new(Var::from_index(ni.index() + self.vertex_count), true)
    }

    fn new_proposition<Solver: SolverInterface>(&self, solver: &mut Solver) -> Lit {
        Lit::new(solver.new_var_default(), true)
    }
}

#[derive(Default)]
pub struct FbasAnalyzer<Cb: Callbacks> {
    fbas: Fbas,
    solver: Solver<Cb>,
    status: SolveStatus,
}

#[derive(Clone, Default, PartialEq)]
pub enum SolveStatus {
    SAT((Vec<NodeIndex>, Vec<NodeIndex>)),
    UNSAT,
    #[default]
    UNKNOWN,
}

impl std::fmt::Debug for SolveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolveStatus::SAT((quorum_a, quorum_b)) => {
                write!(f, "SAT(quorum_a: {:?}, quorum_b: {:?})", quorum_a, quorum_b)
            }
            SolveStatus::UNSAT => write!(f, "UNSAT"),
            SolveStatus::UNKNOWN => write!(f, "UNKNOWN"),
        }
    }
}

impl std::fmt::Display for SolveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as std::fmt::Debug>::fmt(&self, f)
    }
}

impl<Cb: Callbacks> FbasAnalyzer<Cb> {
    pub fn from_quorum_set_map_buf<T: AsRef<[u8]>, I: ExactSizeIterator<Item = T>>(
        nodes: I,
        quorum_set: I,
        cb: Cb,
    ) -> Result<Self, FbasError> {
        let fbas = Fbas::from_quorum_set_map_buf(nodes, quorum_set)?;
        Self::from_fbas(fbas, cb)
    }

    #[cfg(any(feature = "json", test))]
    pub fn from_json_path(path: &str, cb: Cb) -> Result<Self, FbasError> {
        let fbas = Fbas::from_json_path(path)?;
        Self::from_fbas(fbas, cb)
    }

    pub(crate) fn from_fbas(fbas: Fbas, cb: Cb) -> Result<Self, FbasError> {
        let mut analyzer = Self {
            fbas,
            solver: Solver::new(Default::default(), cb),
            status: SolveStatus::UNKNOWN,
        };
        analyzer.construct_formula()?;
        Ok(analyzer)
    }

    fn construct_formula(&mut self) -> Result<(), FbasError> {
        let fbas = &self.fbas;
        let fbas_lits = FbasLitsWrapper::new(fbas.graph.node_count());

        // for each vertex in the graph, we add a variable representing it
        // belonging to quorum A and quorum B
        fbas.graph.node_indices().for_each(|_| {
            self.solver.new_var_default();
            self.solver.new_var_default();
        });
        debug_assert!(self.solver.num_vars() as usize == fbas.graph.node_count() * 2);

        // formula 1: both quorums are non-empty -- at least one validator must
        // exist in each quorum
        let mut quorums_not_empty: (Vec<Lit>, Vec<Lit>) = fbas
            .validators
            .iter()
            .map(|ni| (fbas_lits.in_quorum_a(ni), fbas_lits.in_quorum_b(ni)))
            .collect();
        self.solver.add_clause_reuse(&mut quorums_not_empty.0);
        self.solver.add_clause_reuse(&mut quorums_not_empty.1);

        // formula 2: two quorums do not intersect -- no validator can appear in
        // both quorums
        fbas.validators.iter().for_each(|ni| {
            self.solver.add_clause_reuse(&mut vec![
                !fbas_lits.in_quorum_a(ni),
                !fbas_lits.in_quorum_b(ni),
            ]);
        });

        // formula 3: qset relation for each vertex must be satisfied
        let mut add_clauses_for_quorum_relations =
            |in_quorum: &dyn Fn(&NodeIndex) -> Lit| -> Result<(), FbasError> {
                fbas.graph.node_indices().try_for_each(|ni| {
                    let aq_i = in_quorum(&ni);
                    let nd = fbas
                        .graph
                        .node_weight(ni)
                        .ok_or_else(|| FbasError::InternalError("Node index not found"))?;
                    let threshold = nd.get_threshold();
                    let neighbors = fbas.graph.neighbors(ni);
                    let qset = neighbors.into_iter().combinations(threshold as usize);

                    let mut third_term = vec![];
                    third_term.push(!aq_i);
                    for (_j, q_slice) in qset.enumerate() {
                        // create a new proposition as per Tseitin transformation
                        let xi_j = fbas_lits.new_proposition(&mut self.solver);

                        // this is the second part in the qsat_i^{A} equation
                        let mut neg_pi_j = vec![];
                        neg_pi_j.push(!aq_i);
                        neg_pi_j.push(xi_j);
                        for (_k, elem) in q_slice.iter().enumerate() {
                            // get lit for elem
                            let elit = in_quorum(elem);
                            neg_pi_j.push(!elit);
                            // this is the first part of the equation
                            self.solver.add_clause_reuse(&mut vec![!aq_i, !xi_j, elit]);
                        }
                        self.solver.add_clause_reuse(&mut neg_pi_j);

                        third_term.push(xi_j);
                    }
                    self.solver.add_clause_reuse(&mut third_term);
                    Ok(())
                })
            };

        add_clauses_for_quorum_relations(&|ni| fbas_lits.in_quorum_a(ni))?;
        add_clauses_for_quorum_relations(&|ni| fbas_lits.in_quorum_b(ni))?;
        Ok(())
    }

    pub fn solve(&mut self) -> SolveStatus {
        let mut th = theory::EmptyTheory::new();
        let result = self.solver.solve_limited_th_full(&mut th, &[]);
        self.status = match result {
            SolveResult::Sat(model) => {
                let fbas_lits = FbasLitsWrapper::new(self.fbas.graph.node_count());
                let mut quorum_a = vec![];
                let mut quorum_b = vec![];
                self.fbas.validators.iter().for_each(|ni| {
                    let la = fbas_lits.in_quorum_a(ni);
                    if model.value_lit(la) == lbool::TRUE {
                        quorum_a.push(*ni);
                    }
                    let lb = fbas_lits.in_quorum_b(ni);
                    if model.value_lit(lb) == lbool::TRUE {
                        quorum_b.push(*ni);
                    }
                });
                SolveStatus::SAT((quorum_a, quorum_b))
            }
            SolveResult::Unsat(_) => SolveStatus::UNSAT,
            SolveResult::Unknown(_) => SolveStatus::UNKNOWN,
        };
        self.status.clone()
    }

    pub fn get_potential_split(&self) -> Result<(Vec<String>, Vec<String>), FbasError> {
        match &self.status {
            SolveStatus::SAT((quorum_a, quorum_b)) => {
                let qa_strings = quorum_a
                    .iter()
                    .map(|ni| self.fbas.try_get_validator_string(ni))
                    .collect::<Result<Vec<_>, _>>()?;
                let qb_strings = quorum_b
                    .iter()
                    .map(|ni| self.fbas.try_get_validator_string(ni))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((qa_strings, qb_strings))
            }
            _ => Ok((vec![], vec![])),
        }
    }
}
