//! Exact pinned 26.3 undead sun-burn gate (`scenarios/mobs/undead-sun-burn.json`):
//! NoAI zombies and a skeleton at noon, in the open, helmeted, pumpkin-headed
//! and in water. `Mob.burnUndead` rolls against the light each tick from
//! the mob's own random; burning costs one health a second
//! (`Entity.baseTick`). The scene is open to the sky (full light at noon)
//! except where blocks stand above.
use glam::DVec3;
use minecraftoss_entities::{skeleton::Skeleton, world::EntityWorld, zombie::Zombie};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
impl World for Scene {
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
    fn can_see_sky(&self, (x, y, z): Pos) -> bool {
        !self.0.keys().any(|&(bx, by, bz)| bx == x && bz == z && by > y)
    }
    /// Noon: full sky light under open sky (magic value 1), none below blocks.
    fn light_path_cost(&self, pos: Pos) -> f32 {
        if self.can_see_sky(pos) { 0.5 } else { -0.5 }
    }
}
fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_undead_sun_burn TRACE.jsonl");
    let mut world = EntityWorld::default();
    world.set_monsters_burn(true);
    let mut scene = Scene::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut heads: BTreeMap<String, Option<bool>> = BTreeMap::new();
    let (mut frames, mut burning) = (0, 0);
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "complete" => complete = true,
            "scenario_start" => {
                for command in row["data"]["prepare"].as_array().unwrap() {
                    let parts: Vec<&str> = command.as_str().unwrap().split_whitespace().collect();
                    let n = |i: usize| parts[i].parse::<i32>().unwrap();
                    match parts[0] {
                        "fill" => {
                            for x in n(1)..=n(4) {
                                for y in n(2)..=n(5) {
                                    for z in n(3)..=n(6) {
                                        scene.set_block((x, y, z), Some(Block::new(parts[7])));
                                    }
                                }
                            }
                        }
                        "setblock" => scene.set_block((n(1), n(2), n(3)), Some(Block::new(parts[4]))),
                        _ => {}
                    }
                }
                for command in row["data"]["setup"].as_array().unwrap() {
                    let text = command.as_str().unwrap();
                    let tag = text.split("Tags:[\"").nth(1).unwrap().split('"').next().unwrap().to_owned();
                    let head = text.split("head:{id:\"").nth(1).map(|rest| rest.split('"').next().unwrap().ends_with("_helmet"));
                    heads.insert(tag, head);
                }
            }
            "entity_set_random_seed" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let seed = row["data"]["seed"].as_str().unwrap().parse().unwrap();
                let id = ids[tag];
                if let Some(zombie) = world.zombie_mut(id) {
                    zombie.set_random_seed(seed);
                } else {
                    world.skeleton_mut(id).unwrap().set_random_seed(seed);
                }
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let entities = data["entities"].as_object().unwrap();
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    // Spawn in entity ID order.
                    let mut order: Vec<(&String, &Value)> = entities.iter().map(|(tag, states)| (tag, &states[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let position = DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"]));
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = if e["type"] == "minecraft:skeleton" {
                            let mut skeleton = Skeleton::new(position);
                            skeleton.body.on_ground = e["on_ground"].as_bool().unwrap();
                            skeleton.persistence_required = true;
                            skeleton.head_item = heads[tag];
                            world.spawn_skeleton(skeleton, true)
                        } else {
                            let mut zombie = Zombie::new(position);
                            zombie.body.on_ground = e["on_ground"].as_bool().unwrap();
                            zombie.persistence_required = true;
                            zombie.head_item = heads[tag];
                            world.spawn_zombie(zombie, true)
                        };
                        ids.insert(tag.clone(), id);
                    }
                    continue;
                }
                world.tick_with_players(&mut scene, &[]);
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "tick {tick} game time");
                for (tag, states) in entities {
                    let e = &states[0];
                    let id = ids[tag];
                    let (fire, health, ambient, ticks) = if let Some(z) = world.zombies().iter().find(|z| z.id == id) {
                        (z.zombie.body.fire_ticks, z.zombie.health, z.ambient_sound_time, z.tick_count)
                    } else {
                        let s = world.skeletons().iter().find(|s| s.id == id).unwrap();
                        (s.skeleton.body.fire_ticks, s.skeleton.health, s.ambient_sound_time, s.tick_count)
                    };
                    assert_eq!(e["entity_tick_count"], ticks, "tick {tick} {tag} tick count");
                    assert_eq!(e["remaining_fire_ticks"], fire, "tick {tick} {tag} fire ticks");
                    assert_eq!(f64::from(health).to_bits(), number(&e["health"]).to_bits(), "tick {tick} {tag} health");
                    assert_eq!(e["ambient_sound_time"], ambient, "tick {tick} {tag} ambient time");
                    if fire > 0 {
                        burning += 1;
                    }
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(burning > 0, "some undead burn");
    println!("{frames} exact undead sun-burn frames matched ({burning} burning mob-ticks)");
}
