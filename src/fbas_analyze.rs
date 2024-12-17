use crate::Fbas;
use petgraph::{csr::IndexType, graph::NodeIndex};
use varisat::*;
use std::cell::RefCell;
use itertools::Itertools;


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
    lits_count: RefCell<usize>
}

impl FbasLitsWrapper {
    fn new(vcount: usize) -> Self {
        Self {
            vertex_count: vcount,
            lits_count: RefCell::new(vcount * 2),
        }
    }

    fn in_quorum_a(&self, ni: &NodeIndex) -> Lit {
        Lit::from_index(ni.index(), true)
    }

    fn in_quorum_b(&self, ni: &NodeIndex) -> Lit {
        Lit::from_index(ni.index() + self.vertex_count, true)
    }

    fn new_proposition(&self) -> Lit {
        let lit = Lit::from_index(*self.lits_count.borrow(), true);
        *self.lits_count.borrow_mut() += 1;
        lit
    }
}

pub fn fbas_analyze(fbas: Fbas) -> bool {
    // declare two set of lits, one for quorum A, one for quorum B. 
    // each set contains all vertices of the fbas
    let fbas_lits = FbasLitsWrapper::new(fbas.graph.node_count());
    let mut formula = CnfFormula::new();

    // formula 1: both quorums are non-empty -- at least one validator must exist in each quorum
    let quorums_not_empty: (Vec<Lit>, Vec<Lit>) = fbas.validators.iter().map(|ni| {
        (fbas_lits.in_quorum_a(ni), fbas_lits.in_quorum_b(ni))
    }).collect();
    formula.add_clause(quorums_not_empty.0.as_slice());
    formula.add_clause(quorums_not_empty.1.as_slice());

    // formula 2: two quorums do not intersect -- no validator can appear in both quorums
    fbas.validators.iter().for_each(|ni| {
        formula.add_clause(&[!fbas_lits.in_quorum_a(ni), !fbas_lits.in_quorum_b(ni)]);
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
                let xi_j = fbas_lits.new_proposition();
    
                // this is the second part in the qsat_i^{A} equation
                let mut neg_pi_j = vec![];
                neg_pi_j.push(!aq_i);
                neg_pi_j.push(xi_j);
                for (_k, elem) in q_slice.iter().enumerate() {
                    // get lit for elem                    
                    let elit = in_quorum(elem);
                    neg_pi_j.push(!elit);
                    // this is the first part of the equation
                    formula.add_clause(&[!aq_i, !xi_j, elit]);
                }
                formula.add_clause(neg_pi_j.as_slice());
    
                third_term.push(xi_j);
            }
            formula.add_clause(third_term.as_slice());
        });
    };

    add_clauses_for_quorum_relations(&|ni| fbas_lits.in_quorum_a(ni));
    add_clauses_for_quorum_relations(&|ni| fbas_lits.in_quorum_b(ni));
    
    // solve
    let mut solver = Solver::new();
    solver.add_formula(&formula);
    solver.solve().unwrap()
}

#[cfg(test)]
mod test {
    use crate::{fbas_analyze, Fbas};
    use std::{io::BufRead, str::FromStr};

    #[test]
    fn test() -> std::io::Result<()> { 
        let mut test_cases = vec![];
        for entry in std::fs::read_dir("./tests/test_data/")? {
            let path = entry?.path();
            if let Some(extension) = path.extension() {
                if extension == "json"{
                    let case = path.file_stem().unwrap().to_os_string();
                    test_cases.push(case);
                    let fbas = Fbas::from_json(path.as_os_str().to_str().unwrap()).unwrap();
                    let res = fbas_analyze(fbas);                
                    println!("{res}");
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
                if extension == "dimacs"{
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
            let res = fbas_analyze(fbas);
            println!("{res}");
            
            {
                // Open and read the file line by line
                let file = std::fs::File::open(dimacs_file).expect("Failed to open the DIMACS file");
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
                            break;
                        }
                    }
                }
                assert_eq!(res, expected);
            }
        }
        Ok(())
    }
}

