use json::JsonValue;
use petgraph::graph::{DiGraph, NodeIndex};
use std::fs::File;
use std::io::Read;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
};

const QUORUM_SET_MAX_DEPTH: u32 = 4;

// This is the internal representation of a quorum set. The Qset structure must
// be explicitly specified (by validator's declaration). You can't say my inner
// qset is "another validator's qset". Because of that, the `Qset` structure
// cannot contain a cycle. This is different from transitive qset, which is
// obtained by extending the all dependent validators by including their
// dependent qsets (a validator can only depend on a single qsat, not other
// validators). Such transititive structure is described by the graph in `Fbas`.
// A leaf in a `Qsat` can only contain 1. validator or 2. vacumous qset (qset
// with a threshold but empty validator list and innerqset).
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Default)]
pub struct Qset {
    pub threshold: u32,
    pub validators: BTreeSet<NodeIndex>, // Stores index of validators that have been parsed and already exists in the graph.
    pub inner_qsets: BTreeSet<NodeIndex>, // Stores index of qsets that have been parsed. Because the Qset is parsed in depth-first manner and cannot contain cycles, this is possible.
}

#[derive(Debug)]
pub enum Vertex {
    Validator(String),
    QSet(Qset),
}

impl Vertex {
    pub (crate) fn get_threshold(&self) -> u32 {
        match self {
            Vertex::Validator(_) => 1,
            Vertex::QSet(qset) => qset.threshold,
        }
    }
}

#[derive(Debug)]
pub enum FbasError {
    ParseError,
}

impl std::error::Error for FbasError {}

impl std::fmt::Display for FbasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <FbasError as Debug>::fmt(&self, f)
    }
}

#[derive(Default, Debug)]
pub struct Fbas {
    pub graph: DiGraph<Vertex, ()>,
    pub validators: Vec<NodeIndex>, // nodes of validators in the graph
}

impl Fbas {
    fn add_validator(&mut self, v: &str) -> NodeIndex{
        let idx = self.graph.add_node(Vertex::Validator(v.to_string()));
        self.validators.push(idx);
        idx
    }

    fn process_quorum_set(&mut self, quorum_set: &JsonValue, curr_depth: u32, known_validators: &BTreeMap<&str, NodeIndex>, known_qsets: &mut BTreeMap<Qset, NodeIndex>) -> NodeIndex {
        if curr_depth == QUORUM_SET_MAX_DEPTH {
            panic!("qset exceeds max depth")
        }

        if let JsonValue::Object(quorum_set) = quorum_set {
            let mut qset = Qset::default();

            if let Some(threshold) = quorum_set.get("threshold").and_then(|v| v.as_u32()) {
                qset.threshold = threshold;
                // process validators, this happens after the first pass therefore all known validators already exist
                if let Some(&JsonValue::Array(ref validators)) = quorum_set.get("validators") {
                    for v in validators {
                        let vstr = v.as_str().unwrap();
                        if let Some(vidx) = known_validators.get(vstr) {
                            qset.validators.insert(*vidx);
                        } else {
                            // if a validator string shows up in the qset but it's not declared at the top level
                            // we ingore it. this is mostly likely indicating the validator is offline or undiscoverable.
                            // from consensus point of view they do not exist.
                            eprintln!("Validators {:?} is unknown", vstr);                            
                        }
                    }
                    // assert_eq!(validators.len(), qset.validators.len(), "qset contains duplicate validators");
                } else {
                    eprintln!("validators not found in quorum set");
                }
                // process inner quorum sets
                if let Some(&JsonValue::Array(ref inner_qsets)) = quorum_set.get("innerQuorumSets") {
                    for iqs in inner_qsets{
                        let qidx = Self::process_quorum_set(self, iqs, curr_depth+1, known_validators, known_qsets);
                        qset.inner_qsets.insert(qidx);
                    }
                }
            } else {
                eprintln!("threshold not found in quorum set");
            }

            let idx = if let Some(idx) = known_qsets.get(&qset) {
                *idx
            } else {
                let idx = self.graph.add_node(Vertex::QSet(qset.clone()));
                idx
            };        
            qset.validators.iter().for_each(|vi| {let _ = self.graph.update_edge(idx, *vi, ()); });
            qset.inner_qsets.iter().for_each(|qi| { let _ = self.graph.update_edge(idx, *qi, ()); });
            known_qsets.insert(qset, idx);
            
            idx
        } else {
            panic!("malformed quorum set")                        
        }
    }

    pub fn from_json(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        println!("{path}");
        let mut fbas = Fbas::default();

        // Read the JSON file into a string
        let mut file = File::open(path)?;
        let mut data = String::new();
        file.read_to_string(&mut data)?;

        // Parse the JSON
        if let JsonValue::Array(nodes) = &json::parse(&data)? {
            // first pass: collect all validators
            let mut known_validators = BTreeMap::new();
            for nd in nodes.iter() {
                if let Some(public_key) = nd["publicKey"].as_str() {
                    if !known_validators.contains_key(public_key) {
                        let ni = fbas.add_validator(public_key);
                        known_validators.insert(public_key, ni);
                    } else {
                        panic!("duplicate publicKey");
                    }
                } else {
                    panic!("publicKey not found in node");
                }                
            }

            // second pass: parse qsets and make connections
            let mut known_qsets = BTreeMap::<Qset, NodeIndex>::new();
            for nd in nodes.iter() {
                // TODO: repeat above validity check
                let pk = nd["publicKey"].as_str().unwrap(); // we've already checked above key exists
                let v_idx = known_validators.get(pk).unwrap();

                let q_idx = fbas.process_quorum_set(&nd["quorumSet"], 0, &known_validators, &mut known_qsets);
                let _ = fbas.graph.add_edge(*v_idx, q_idx, ());
            }
        } else {
            panic!("Expected root element to be an array");
        }

        // Write DOT output to a file
        // let dot_file = "graph.dot";
        // let mut file = File::create(dot_file)?;
        // write!(file, "{:?}", Dot::with_config(&fbas.graph, &[]))?;
        // Use Graphviz to render manually or with a system command
        // println!("DOT file created: {}", dot_file);

        Ok(fbas)
    }
}

#[test]
fn test() {
    Fbas::from_json("./tests/test_data/top_tier.json").unwrap();
}
