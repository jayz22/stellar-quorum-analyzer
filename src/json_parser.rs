use crate::fbas::{InternalScpQuorumSet, QuorumSetMap};
use json::{object::Object, JsonValue};
use std::{fs::File, io::Read, rc::Rc};

pub(crate) fn quorum_set_map_from_json(
    path: &str,
) -> Result<QuorumSetMap, Box<dyn std::error::Error>> {
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
