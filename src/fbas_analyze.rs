use crate::Fbas;
use batsat::{
    interface::SolveResult, intmap::AsIndex, lbool, theory, Callbacks, Lit, Solver,
    SolverInterface, Var,
};
use itertools::Itertools;
use petgraph::{csr::IndexType, graph::NodeIndex};

// Two imaginary quorums A and B, and we have FBAS system with V vertices. Note
// the a quorum contain validators, whereas a vertex can be either a validator
// or a qset. The relation of each vertice being in each quorum is represented
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
// dependencies. I.e. any threshold number out of all sucessors needs to be in
// the quorum as well.
//
// These three constrains must all be hold.
//
// Before we send these constrains to a SAT solver, they must be transformed
// into conjunction norm form (CNF). Thus a portion of work is to perform the
// Tseitin transformation on the constrains into "AND of ORs", which also
// introdues additional imaginary atomics. Once the solver produces a
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
        match self {
            SolveStatus::SAT((quorum_a, quorum_b)) => {
                write!(f, "SAT - Split found:\nQuorum A: {:#?}\nQuorum B: {:#?}", quorum_a, quorum_b)
            }
            SolveStatus::UNSAT => write!(f, "UNSAT - No split exists"),
            SolveStatus::UNKNOWN => write!(f, "UNKNOWN - Solver status unknown"),
        }
    }
}

impl<Cb: Callbacks> FbasAnalyzer<Cb> {
    pub fn new(fbas: Fbas, cb: Cb) -> Result<Self, Box<dyn std::error::Error>> {
        let mut analyzer = Self {
            fbas,
            solver: Solver::new(Default::default(), cb),
            status: SolveStatus::UNKNOWN,
        };
        analyzer.construct_formula()?;
        Ok(analyzer)
    }

    fn construct_formula(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let fbas = &self.fbas;
        let fbas_lits = FbasLitsWrapper::new(fbas.graph.node_count());

        // for each vertice in the graph, we add a variable representing it belonging to quorum A and quorum B
        fbas.graph.node_indices().for_each(|_| {
            self.solver.new_var_default();
            self.solver.new_var_default();
        });
        debug_assert!(self.solver.num_vars() as usize == fbas.graph.node_count() * 2);

        // formula 1: both quorums are non-empty -- at least one validator must exist in each quorum
        let mut quorums_not_empty: (Vec<Lit>, Vec<Lit>) = fbas
            .validators
            .iter()
            .map(|ni| (fbas_lits.in_quorum_a(ni), fbas_lits.in_quorum_b(ni)))
            .collect();
        self.solver.add_clause_reuse(&mut quorums_not_empty.0);
        self.solver.add_clause_reuse(&mut quorums_not_empty.1);

        // formula 2: two quorums do not intersect -- no validator can appear in both quorums
        fbas.validators.iter().for_each(|ni| {
            self.solver.add_clause_reuse(&mut vec![
                !fbas_lits.in_quorum_a(ni),
                !fbas_lits.in_quorum_b(ni),
            ]);
        });

        // formula 3: qset relation for each vertice must be satisfied
        let mut add_clauses_for_quorum_relations = |in_quorum: &dyn Fn(&NodeIndex) -> Lit| {
            fbas.graph.node_indices().for_each(|ni| {
                let aq_i = in_quorum(&ni);
                let nd = fbas.graph.node_weight(ni).unwrap();
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
            });
        };

        add_clauses_for_quorum_relations(&|ni| fbas_lits.in_quorum_a(ni));
        add_clauses_for_quorum_relations(&|ni| fbas_lits.in_quorum_b(ni));
        Ok(())
    }

    pub fn solve(&mut self) -> SolveStatus {

        println!("callback stop condition: {}", self.solver.cb().stop());

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
            SolveResult::Unsat(_) => {
                SolveStatus::UNSAT
            }
            SolveResult::Unknown(_) => {
                SolveStatus::UNKNOWN
            }
        };
        self.status.clone()
    }

    pub fn get_potential_split(&self) -> (Vec<String>, Vec<String>) {
        if let SolveStatus::SAT((quorum_a, quorum_b)) = &self.status {
            let qa_strings = quorum_a
                .iter()
                .map(|ni| self.fbas.get_validator(ni).unwrap().clone())
                .collect();
            let qb_strings = quorum_b
                .iter()
                .map(|ni| self.fbas.get_validator(ni).unwrap().clone())
                .collect();
            (qa_strings, qb_strings)
        } else {
            (vec![], vec![])
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{Fbas, FbasAnalyzer, SolveStatus};
    use batsat::callbacks::{AsyncInterrupt, Basic};
    use std::{io::BufRead, str::FromStr};

    #[test]
    fn test_solver_interrupt() -> Result<(), Box<dyn std::error::Error>> {
        let json_file = std::path::PathBuf::from("./tests/test_data/random/almost_symmetric_network_16_orgs_delete_prob_factor_1.json");
        let fbas = Fbas::from_json(json_file.as_os_str().to_str().unwrap()).unwrap();
        let cb = AsyncInterrupt::default();
        let handle = cb.get_handle();
        let mut solver = FbasAnalyzer::new(fbas, cb)?;

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_micros(100));
            handle.interrupt_async();
        });

        let res = solver.solve();
        assert_eq!(res, SolveStatus::UNKNOWN);
        Ok(())
    }

    #[test]
    fn test() -> std::io::Result<()> {
        let mut test_cases = vec![];
        for entry in std::fs::read_dir("./tests/test_data/")? {
            let path = entry?.path();
            if let Some(extension) = path.extension() {
                if extension == "json" {
                    let case = path.file_stem().unwrap().to_os_string();
                    test_cases.push(case);
                    let fbas = Fbas::from_json(path.as_os_str().to_str().unwrap()).unwrap();
                    let mut solver = FbasAnalyzer::new(fbas, Basic::default()).unwrap();
                    let res = solver.solve();
                    println!("{:?}", res);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn test_random_data() -> std::io::Result<()> {
        let mut test_cases = vec![];
        let dir_path = std::ffi::OsString::from_str("./tests/test_data/random/").unwrap();
        for entry in std::fs::read_dir("./tests/test_data/random/")? {
            let path = entry?.path();
            if let Some(extension) = path.extension() {
                if extension == "dimacs" {
                    let case = path.file_stem().unwrap().to_os_string();
                    test_cases.push(case);
                }
            }
        }

        for case in test_cases {
            let mut json_file = dir_path.clone();
            json_file.push(case.clone());
            json_file.push(".json");

            let mut dimacs_file = dir_path.clone();
            dimacs_file.push(case.clone());
            dimacs_file.push(".dimacs");

            let fbas = Fbas::from_json(json_file.as_os_str().to_str().unwrap()).unwrap();
            let mut solver = FbasAnalyzer::new(fbas, Basic::default()).unwrap();
            let res = solver.solve();
            {
                // Open and read the file line by line
                let file =
                    std::fs::File::open(dimacs_file).expect("Failed to open the DIMACS file");
                let reader = std::io::BufReader::new(file);

                // Look for the result comment line
                let mut expected = false;
                for line in reader.lines() {
                    let line = line.expect("Failed to read line");
                    if line.starts_with("c") {
                        if line.contains("UNSATISFIABLE") {
                            expected = false;
                            break;
                        } else if line.contains("SATISFIABLE") {
                            expected = true;
                            let (qa, qb) = solver.get_potential_split();
                            println!("quorum a: {:?}, quorum b: {:?}", qa, qb);
                            break;
                        }
                    }
                }
                let is_sat = matches!(res, SolveStatus::SAT(_));
                assert_eq!(is_sat, expected);
            }
        }
        Ok(())
    }
}
