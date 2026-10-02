//! Narrow replay of scenarios/honeycomb-hand-use.json, using the pinned waxable catalog.
use minecraftoss_player::{wax, Block};
use serde_json::{json, Value};

fn block(block: &Block) -> Value {
    json!({"id": block.id, "properties": block.properties})
}

fn main() {
    let creative = std::env::args().any(|arg| arg == "--creative");
    let nonbuilding = std::env::args().any(|arg| arg == "--nonbuilding");
    if nonbuilding {
        let copper = Block::new("minecraft:copper_block");
        let mut records = Vec::new();
        for tick in 0..=5 {
            if tick == 2 || tick == 3 {
                let x = tick - 2;
                records.push(json!({
                    "tick": tick,
                    "type": "honeycomb_use",
                    "data": {"pos":[x,81,0], "result":"pass", "remaining_count":2,
                             "game_mode": if tick == 2 { "adventure" } else { "spectator" },
                             "instabuild":false},
                }));
            }
            records.push(json!({
                "tick": tick,
                "type": "snapshot",
                "data": {
                    "blocks": {"0,81,0":block(&copper), "1,81,0":block(&copper)},
                    "sound_events": [],
                    "level_events": [],
                }
            }));
        }
        println!(
            "{}",
            serde_json::to_string(&records).expect("serializable replay")
        );
        return;
    }
    let mut copper = Block::new("minecraft:copper_block");
    let mut stairs = Block::new("minecraft:oxidized_cut_copper_stairs")
        .with("facing", "west")
        .with("half", "top")
        .with("shape", "straight")
        .with("waterlogged", "false");
    let mut records = Vec::new();
    for tick in 0..=if creative { 4 } else { 7 } {
        let mut sounds = Vec::new();
        let mut events = Vec::new();
        if let Some((x, current)) = if creative {
            (tick == 2).then_some((0, &mut copper))
        } else {
            match tick {
                2 | 3 => Some((0, &mut copper)),
                4 => Some((1, &mut stairs)),
                _ => None,
            }
        } {
            let waxed = wax::waxed_block(current);
            let success = waxed.is_some();
            if let Some(next) = waxed {
                *current = next;
                sounds.push(json!({
                    "id": "minecraft:item.honeycomb.wax_on",
                    "category": "block",
                    "position": [x,81,0],
                    "volume_bits": "3f800000",
                    "pitch_bits": "3f800000",
                }));
                events.push(json!({"id":3003, "position":[x,81,0], "param":0}));
            }
            records.push(json!({
                "tick": tick,
                "type": "honeycomb_use",
                "data": {"pos":[x,81,0], "result": if success { "success" } else { "pass" },
                         "remaining_count": if success && !creative { 1 } else { 2 },
                         "game_mode": if creative { "creative" } else { "survival" },
                         "instabuild": creative},
            }));
        }
        let blocks = if creative {
            json!({"0,81,0":block(&copper)})
        } else {
            json!({"0,81,0":block(&copper), "1,81,0":block(&stairs)})
        };
        records.push(json!({
            "tick": tick,
            "type": "snapshot",
            "data": {
                "blocks": blocks,
                "sound_events": sounds,
                "level_events": events,
            }
        }));
    }
    println!(
        "{}",
        serde_json::to_string(&records).expect("serializable replay")
    );
}
