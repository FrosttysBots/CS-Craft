//! Shared by the mob gates: the pinned block-state catalog
//! (`artifacts/block-state-catalog/26.3.json`, exported by the harness)
//! for scenes with odd-shaped blocks: each state's collision boxes and its
//! solidity and land-pathfinding flags, found by block and properties
//! (missing properties take the block's default state's).
#![allow(dead_code)]
use minecraftoss_player::{path_type::PathType, Block};
use serde_json::Value;
use std::collections::HashMap;

/// What a gate's scene needs of a block state.
#[derive(Clone, Debug)]
pub struct StateInfo {
    /// `getCollisionShape` in an empty context, in block-local units.
    pub collision: Vec<[f64; 6]>,
    /// `isPathfindable(LAND)`.
    pub pathfindable: bool,
    /// `isSolid` (the legacy flag).
    pub solid: bool,
    /// `isSolidRender`.
    pub solid_render: bool,
    /// Its `SoundType`'s step event (namespace dropped), volume and pitch.
    pub step: Option<(String, f32, f32)>,
    /// `isSuffocating`.
    pub suffocating: bool,
}

pub struct Catalog {
    states: HashMap<String, StateInfo>,
    defaults: HashMap<String, Vec<(String, String)>>,
}

fn key(id: &str, properties: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = properties.iter().collect();
    sorted.sort();
    let inner: Vec<String> = sorted.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{id}[{}]", inner.join(","))
}

fn bits(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

impl Catalog {
    pub fn load(path: &str) -> Catalog {
        let root: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_else(|e| panic!("block-state catalog {path}: {e}"))).unwrap();
        let shapes: Vec<Vec<[f64; 6]>> = root["shapes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|shape| match shape {
                Value::String(s) if s == "empty" => Vec::new(),
                Value::String(s) if s == "full" => vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]],
                Value::Array(boxes) => boxes
                    .iter()
                    .map(|b| {
                        let b = b.as_array().unwrap();
                        [bits(&b[0]), bits(&b[1]), bits(&b[2]), bits(&b[3]), bits(&b[4]), bits(&b[5])]
                    })
                    .collect(),
                other => panic!("unexpected shape {other}"),
            })
            .collect();
        let sounds: Vec<(String, f32, f32)> = root["sound_types"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let float = |k: &str| f32::from_bits(u32::from_str_radix(t[k].as_str().unwrap(), 16).unwrap());
                (t["step"].as_str().unwrap().trim_start_matches("minecraft:").to_owned(), float("volume_bits"), float("pitch_bits"))
            })
            .collect();
        let mut states = HashMap::new();
        let mut defaults = HashMap::new();
        for state in root["states"].as_array().unwrap() {
            let id = state["block"].as_str().unwrap().to_owned();
            let properties: Vec<(String, String)> = state["properties"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (p[0].as_str().unwrap().to_owned(), p[1].as_str().unwrap().to_owned()))
                .collect();
            if state["default"].as_bool() == Some(true) {
                defaults.insert(id.clone(), properties.clone());
            }
            let info = StateInfo {
                collision: shapes[state["collision"].as_u64().unwrap() as usize].clone(),
                pathfindable: state["pathfindable_land"].as_bool().unwrap(),
                solid: state["legacy_solid"].as_bool().unwrap(),
                solid_render: state["solid_render"].as_bool().unwrap(),
                step: state["sound_type"].as_u64().map(|i| sounds[i as usize].clone()),
                suffocating: state["suffocating"].as_bool().unwrap(),
            };
            states.insert(key(&id, &properties), info);
        }
        Catalog { states, defaults }
    }

    /// The state a block names (its properties over its default's).
    pub fn get(&self, block: &Block) -> Option<&StateInfo> {
        let mut properties = self.defaults.get(&block.id)?.clone();
        for (k, v) in &block.properties {
            if let Some(slot) = properties.iter_mut().find(|(name, _)| name == k) {
                slot.1 = v.clone();
            }
        }
        self.states.get(&key(&block.id, &properties))
    }

    /// `WalkNodeEvaluator.getPathTypeFromState` with the state's own
    /// pathfinding flag: after the named special cases, a state that is
    /// not pathfindable on land blocks, water floats, the rest is open.
    pub fn path_type(&self, block: Option<&Block>) -> PathType {
        let named = minecraftoss_player::path_type::path_type_of_block(block);
        let Some(block) = block else { return named };
        match named {
            PathType::Open | PathType::Blocked | PathType::Water => match self.get(block) {
                Some(info) if !info.pathfindable => PathType::Blocked,
                Some(_) if named == PathType::Water => PathType::Water,
                Some(_) => PathType::Open,
                None => named,
            },
            other => other,
        }
    }
}
