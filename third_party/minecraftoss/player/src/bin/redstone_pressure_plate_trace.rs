//! Replay the stationary-item plate fixture against the authored block engine.
use minecraftoss_player::{
    redstone::{PlateEntity, PlateMaterial, RedstoneEngine, UseSound},
    Block, Pos, World,
};
use std::collections::BTreeMap;

#[derive(Default)]
struct Circuit(BTreeMap<Pos, Block>);
impl World for Circuit {
    fn block(&self, pos: Pos) -> Option<Block> {
        self.0.get(&pos).cloned()
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        if let Some(block) = block {
            self.0.insert(pos, block);
        } else {
            self.0.remove(&pos);
        }
    }
}

fn main() {
    let weighted = std::env::args().any(|arg| arg == "--weighted");
    let support = std::env::args().any(|arg| arg == "--support");
    let positions: Vec<Pos> = if support {
        vec![(0, 80, 0), (0, 81, 0), (1, 81, 0)]
    } else {
        vec![(0, 81, 0), (1, 81, 0), (4, 81, 0), (5, 81, 0)]
    };
    let mut world = Circuit::default();
    for x in -2..=7 {
        for z in -2..=2 {
            world.set_block((x, 80, z), Some(Block::new("minecraft:stone")));
        }
    }
    let first = if support { 1 } else { 0 };
    world.set_block(
        positions[first],
        Some(if weighted {
            Block::new("minecraft:light_weighted_pressure_plate").with("power", "0")
        } else {
            Block::new("minecraft:oak_pressure_plate").with("powered", "false")
        }),
    );
    world.set_block(
        positions[first + 1],
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    if !support {
        world.set_block(
            positions[2],
            Some(if weighted {
                Block::new("minecraft:heavy_weighted_pressure_plate").with("power", "0")
            } else {
                Block::new("minecraft:stone_pressure_plate").with("powered", "false")
            }),
        );
        world.set_block(
            positions[3],
            Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
        );
    }
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=if support {
        12
    } else if weighted {
        30
    } else {
        45
    } {
        if tick > 0 {
            if support && tick == 4 {
                world.set_block((0, 80, 0), None);
                engine.changed(&mut world, &[(0, 80, 0)], tick - 1);
            }
            let entities = if (2..if support {
                13
            } else if weighted {
                15
            } else {
                7
            })
                .contains(&tick)
            {
                let centers: &[f64] = if support {
                    &[0.5]
                } else if weighted {
                    &[0.2, 0.8, 4.2, 4.8]
                } else {
                    &[0.5, 4.5]
                };
                centers
                    .iter()
                    .map(|&x| PlateEntity {
                        min: [x - 0.125, 81.1, 0.375],
                        max: [x + 0.125, 81.35, 0.625],
                        living: false,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            engine.update_pressure_plate_entities(&mut world, &entities, tick);
            engine.tick(&mut world, tick);
        }
        let sounds: Vec<_> = engine
            .take_sounds()
            .into_iter()
            .map(|(pos, sound)| {
                let UseSound::PressurePlate { material, powered } = sound else {
                    panic!("unexpected sound")
                };
                let material = match material {
                    PlateMaterial::Wooden => "wooden",
                    PlateMaterial::Stone => "stone",
                    PlateMaterial::Metal => "metal",
                };
                let action = if powered { "on" } else { "off" };
                serde_json::json!({
                    "id": format!("minecraft:block.{material}_pressure_plate.click_{action}"),
                    "category": "block", "position": [pos.0,pos.1,pos.2],
                    "volume_bits": "3f800000", "pitch_bits": "3f800000"
                })
            })
            .collect();
        let scheduled_ticks: Vec<_> = engine
            .take_executed_ticks()
            .into_iter()
            .map(|(pos, id)| serde_json::json!({"block": id, "position": [pos.0,pos.1,pos.2]}))
            .collect();
        let blocks: Vec<_> = positions
            .iter()
            .map(|&pos| {
                if let Some(block) = world.block(pos) {
                    serde_json::json!({"id": block.id, "properties": block.properties})
                } else {
                    serde_json::json!({"id": "minecraft:air", "properties": {}})
                }
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({"tick":tick,"blocks":blocks,"sound_events":sounds,"scheduled_tick_events":scheduled_ticks})
        );
    }
}
