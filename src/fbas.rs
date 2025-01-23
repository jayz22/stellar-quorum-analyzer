use json::{object::Object, JsonValue};
use petgraph::graph::{DiGraph, NodeIndex};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    fs::File,
    io::Read,
    rc::Rc,
};
use stellar_xdr::curr::{Limits, NodeId, PublicKey, ReadXdr, ScpQuorumSet};

const QUORUM_SET_MAX_DEPTH: u32 = 4;

pub type QuorumSetMap = BTreeMap<String, Rc<InternalScpQuorumSet>>;

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

/// Same as ScpQuorumSet except it identifies validators with String instead of `NodeId`,
/// because we want to make it easier for testing by allowing nodes to be random strings
/// instead of requiring valid stellar strkeys

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct InternalScpQuorumSet {
    pub threshold: u32,
    pub validators: Vec<String>,
    pub inner_sets: Vec<InternalScpQuorumSet>,
}

#[derive(Debug)]
pub enum Vertex {
    Validator(String),
    QSet(Qset),
}

impl Vertex {
    pub(crate) fn get_threshold(&self) -> u32 {
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

impl From<ScpQuorumSet> for InternalScpQuorumSet {
    fn from(qset: ScpQuorumSet) -> Self {
        InternalScpQuorumSet {
            threshold: qset.threshold,
            validators: qset
                .validators
                .iter()
                .map(|node_id| match &node_id.0 {
                    PublicKey::PublicKeyTypeEd25519(key) => {
                        stellar_strkey::ed25519::PublicKey(key.0).to_string()
                    }
                })
                .collect(),
            inner_sets: qset
                .inner_sets
                .iter()
                .map(|qs| InternalScpQuorumSet::from(qs.clone()))
                .collect(),
        }
    }
}

#[derive(Default, Debug)]
pub struct Fbas {
    pub graph: DiGraph<Vertex, ()>,
    pub validators: Vec<NodeIndex>, // nodes of validators in the graph
}

impl Fbas {
    fn add_validator(&mut self, v: String) -> NodeIndex {
        let idx = self.graph.add_node(Vertex::Validator(v));
        self.validators.push(idx);
        idx
    }

    pub(crate) fn get_validator(&self, ni: &NodeIndex) -> Option<&String> {
        match self.graph.node_weight(*ni) {
            Some(Vertex::Validator(v)) => Some(v),
            _ => None,
        }
    }

    fn from_quorum_set_map(qsm: QuorumSetMap) -> Result<Self, Box<dyn std::error::Error>> {
        let mut fbas = Fbas::default();
        let mut known_validators = BTreeMap::new();
        let mut known_qsets = BTreeMap::new();

        // First pass: add all validators
        for (node_str, _) in qsm.iter() {
            let idx = fbas.add_validator(node_str.clone());
            known_validators.insert(node_str, idx);
        }

        // Second pass: process quorum sets and create connections
        for (node_str, qset) in qsm.iter() {
            let v_idx = known_validators.get(node_str).unwrap();
            let q_idx =
                fbas.process_scp_quorum_set(qset, 0, &known_validators, &mut known_qsets)?;
            let _ = fbas.graph.add_edge(*v_idx, q_idx, ());
        }

        Ok(fbas)
    }

    fn process_scp_quorum_set(
        &mut self,
        qset: &InternalScpQuorumSet,
        curr_depth: u32,
        known_validators: &BTreeMap<&String, NodeIndex>,
        known_qsets: &mut BTreeMap<Qset, NodeIndex>,
    ) -> Result<NodeIndex, Box<dyn std::error::Error>> {
        if curr_depth == QUORUM_SET_MAX_DEPTH {
            return Err("qset exceeds max depth".into());
        }

        let mut new_qset = Qset::default();
        new_qset.threshold = qset.threshold;

        // Add validators
        for validator in &qset.validators {
            if let Some(&idx) = known_validators.get(validator) {
                new_qset.validators.insert(idx);
            } else {
                eprintln!("Validator {} is unknown", validator);
            }
        }

        // Process inner quorum sets
        for inner_qset in &qset.inner_sets {
            let qidx = self.process_scp_quorum_set(
                inner_qset,
                curr_depth + 1,
                known_validators,
                known_qsets,
            )?;
            new_qset.inner_qsets.insert(qidx);
        }

        // Create or reuse the quorum set node
        let idx = if let Some(&idx) = known_qsets.get(&new_qset) {
            idx
        } else {
            let idx = self.graph.add_node(Vertex::QSet(new_qset.clone()));
            known_qsets.insert(new_qset.clone(), idx);
            idx
        };

        // Add edges
        new_qset.validators.iter().for_each(|vi| {
            let _ = self.graph.update_edge(idx, *vi, ());
        });
        new_qset.inner_qsets.iter().for_each(|qi| {
            let _ = self.graph.update_edge(idx, *qi, ());
        });

        Ok(idx)
    }

    pub fn from_quorum_set_map_buf<T: AsRef<[u8]>, I: ExactSizeIterator<Item = T>>(
        nodes: I,
        quorum_set: I,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        assert_eq!(nodes.len(), quorum_set.len());
        let mut quorum_set_map = QuorumSetMap::new();

        for (node_buf, qset_buf) in nodes.zip(quorum_set) {
            let node = NodeId::from_xdr(node_buf, Limits::none())?;
            let node_str = match &node.0 {
                PublicKey::PublicKeyTypeEd25519(key) => {
                    stellar_strkey::ed25519::PublicKey(key.0).to_string()
                }
            };
            if !qset_buf.as_ref().is_empty() {
                let qset = ScpQuorumSet::from_xdr(qset_buf, Limits::none())?;
                quorum_set_map.insert(node_str, Rc::new(qset.into()));
            } else {
                eprintln!("Validator {} is unknown", node_str);
            }
        }

        Self::from_quorum_set_map(quorum_set_map)
    }

    pub fn from_json(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let quorum_set_map = quorum_set_map_from_json(path)?;
        Self::from_quorum_set_map(quorum_set_map)
    }
}

fn quorum_set_map_from_json(path: &str) -> Result<QuorumSetMap, Box<dyn std::error::Error>> {
    let mut file = File::open(path)?;
    let mut data = String::new();
    file.read_to_string(&mut data)?;
    let json_data = json::parse(&data)?;

    match json_data {
        JsonValue::Object(root) => try_parse_quorum_set_map_from_json_regular(root),
        JsonValue::Array(nodes) => try_parse_quorum_set_map_from_stellarbeats_json(nodes),
        _ => Err("root is neither an object nor an array".into()),
    }
}

fn try_parse_quorum_set_map_from_json_regular(
    root: Object,
) -> Result<QuorumSetMap, Box<dyn std::error::Error>> {
    let nodes = match root.get("nodes") {
        Some(JsonValue::Array(nodes)) => nodes,
        _ => return Err("nodes field missing or not an array".into()),
    };

    let mut quorum_map = QuorumSetMap::new();
    for node in nodes {
        let node = match node {
            JsonValue::Object(n) => n,
            _ => return Err("node is not an object".into()),
        };

        let public_key = node
            .get("node")
            .and_then(|n| n.as_str())
            .ok_or("node field missing or not a string")?
            .to_string();

        let qset = parse_internal_quorum_set(&node["qset"])?;
        quorum_map.insert(public_key, Rc::new(qset));
    }

    Ok(quorum_map)
}

fn parse_internal_quorum_set(
    json_qset: &JsonValue,
) -> Result<InternalScpQuorumSet, Box<dyn std::error::Error>> {
    let threshold = json_qset["t"]
        .as_u32()
        .ok_or("threshold field missing or not a number")?;

    let v = match &json_qset["v"] {
        JsonValue::Array(v) => v,
        _ => return Err("v field missing or not an array".into()),
    };

    let mut validators = vec![];
    let mut inner_sets = vec![];

    for item in v {
        match item {
            JsonValue::String(validator) => {
                validators.push(validator.to_string());
            }
            JsonValue::Object(obj) if obj.get("t").is_some() => {
                inner_sets.push(parse_internal_quorum_set(item)?);
            }
            _ => {
                return Err(
                    "validator entry must be either a string (PublicKey) or an object (QuorumSet)"
                        .into(),
                )
            }
        }
    }

    Ok(InternalScpQuorumSet {
        threshold,
        validators,
        inner_sets,
    })
}

fn parse_stellarbeats_internal_quorum_set(
    json_qset: &JsonValue,
) -> Result<InternalScpQuorumSet, Box<dyn std::error::Error>> {
    let threshold = json_qset["threshold"]
        .as_u32()
        .ok_or("threshold field missing or not a number")?;

    let mut validators = vec![];
    let mut inner_sets = vec![];

    match &json_qset["validators"] {
        JsonValue::Array(validator_arr) => {
            for validator in validator_arr {
                match validator.as_str() {
                    Some(validator_str) => validators.push(validator_str.to_string()),
                    None => return Err("validator entry must be a string".into()),
                }
            }
        }
        _ => return Err("validators field missing or not an array".into()),
    }

    match &json_qset["innerQuorumSets"] {
        JsonValue::Array(inner_arr) => {
            for inner_qset in inner_arr {
                inner_sets.push(parse_stellarbeats_internal_quorum_set(inner_qset)?);
            }
        }
        _ => return Err("innerQuorumSets field missing or not an array".into()),
    }

    Ok(InternalScpQuorumSet {
        threshold,
        validators,
        inner_sets,
    })
}

fn try_parse_quorum_set_map_from_stellarbeats_json(
    nodes: Vec<JsonValue>,
) -> Result<QuorumSetMap, Box<dyn std::error::Error>> {
    let mut quorum_map = QuorumSetMap::new();
    for node in nodes {
        let node = match node {
            JsonValue::Object(n) => n,
            _ => return Err("node is not an object".into()),
        };

        let public_key = node
            .get("publicKey")
            .and_then(|n| n.as_str())
            .ok_or("publicKey field missing or not a string")?
            .to_string();

        let qset = parse_stellarbeats_internal_quorum_set(&node["quorumSet"])?;
        quorum_map.insert(public_key, Rc::new(qset));
    }

    Ok(quorum_map)
}

#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;
    use stellar_strkey::ed25519::PublicKey as StrKeyPublicKey;

    #[test]
    fn test_parse_quorum_set_map_from_json() {
        let quorum_map = quorum_set_map_from_json("./tests/test_data/random/almost_symmetric_network_6_orgs_delete_prob_factor_3_for_stellar_core.json").unwrap();
        assert_eq!(quorum_map.len(), 18);

        // Test parsing of a specific node's quorum set
        let test_key =
            StrKeyPublicKey::from_str("GARHWC6Y4WNGLKCAC7SCFFLEV5GKTKB2AHVIA6C7SU5WLJTDW5W3MPHX")
                .unwrap();
        let test_node_id = test_key.to_string();

        let test_qset = quorum_map.get(&test_node_id).unwrap();
        assert_eq!(test_qset.threshold, 3);
        assert_eq!(test_qset.inner_sets.len(), 3);

        // Verify first inner set in detail
        let first_inner = &test_qset.inner_sets[0];
        assert_eq!(first_inner.threshold, 2);
        assert_eq!(first_inner.validators.len(), 3);

        // Check specific validator IDs in first inner set
        let expected_validators = [
            "GARHWC6Y4WNGLKCAC7SCFFLEV5GKTKB2AHVIA6C7SU5WLJTDW5W3MPHX",
            "GCJIDPIMNOJU4PASPDEHKLQWG2KAM45NNAUEQVY33XMYGAMSYICOK4H4",
            "GDRTHCOC6K6GAT3LNO7L2PQZHM7E3JDUHHSNRKSTH53A2AHBM6WZOUOC",
        ];

        for (i, expected_key) in expected_validators.iter().enumerate() {
            assert_eq!(&first_inner.validators[i], expected_key);
        }
    }

    #[test]
    fn test_parse_quorum_set_map_from_stellarbeats_json() {
        let quorum_map = quorum_set_map_from_json("./tests/test_data/top_tier.json").unwrap();

        // Test parsing of a specific node's quorum set
        let test_key =
            StrKeyPublicKey::from_str("GD6SZQV3WEJUH352NTVLKEV2JM2RH266VPEM7EH5QLLI7ZZAALMLNUVN")
                .unwrap();
        let test_node_id = test_key.to_string();

        let test_qset = quorum_map.get(&test_node_id).unwrap();
        assert_eq!(test_qset.threshold, 5);
        assert!(test_qset.validators.is_empty());
        assert_eq!(test_qset.inner_sets.len(), 7);

        // Verify first inner set
        let first_inner = &test_qset.inner_sets[0];
        assert_eq!(first_inner.threshold, 2);
        assert_eq!(first_inner.validators.len(), 3);
        assert!(first_inner.inner_sets.is_empty());

        // Check specific validator in first inner set
        let expected_validator = "GAAV2GCVFLNN522ORUYFV33E76VPC22E72S75AQ6MBR5V45Z5DWVPWEU";
        assert_eq!(&first_inner.validators[0], expected_validator);
    }
}
