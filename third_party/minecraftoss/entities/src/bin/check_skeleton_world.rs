//! Full shared-world skeleton bow fixture gate, pinned Java 26.3.
use glam::DVec3;
use minecraftoss_entities::{
    chicken::Chicken, cow::Cow, skeleton::Skeleton, tempt::PlayerCandidate, villager::Villager,
    world::EntityWorld, zombie::Zombie,
};
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
}
fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}
fn exact(value: &Value, actual: f64, tick: i64, field: &str) {
    assert_eq!(
        number(value).to_bits(),
        actual.to_bits(),
        "tick {tick} {field}"
    );
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_skeleton_world TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut players = Vec::new();
    let mut skeleton_id = None;
    let mut villager_id = None;
    let mut cow_id = None;
    let mut zombie_id = None;
    let mut chicken_id = None;
    let mut frames = 0;
    let mut expected_frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => {
                let case = &row["data"]["suite"]["scenarios"][0];
                assert!(
                    case["id"] == "skeleton_player_bow_pursuit"
                        || case["id"] == "skeleton_arrow_block_impact"
                        || case["id"] == "skeleton_arrow_block_removed"
                        || case["id"] == "skeleton_arrow_villager_hit"
                        || case["id"] == "skeleton_arrow_cow_hit"
                        || case["id"] == "skeleton_arrow_zombie_hit"
                        || case["id"] == "skeleton_arrow_chicken_hit"
                        || case["id"] == "skeleton_arrow_rejected_hit"
                );
                expected_frames = case["ticks"].as_u64().unwrap() as usize;
            }
            "command" if row["data"]["phase"] == "action" => {
                let command = row["data"]["command"].as_str().unwrap();
                let parts: Vec<_> = command.split_whitespace().collect();
                if parts.first() == Some(&"setblock") {
                    assert_eq!(parts.len(), 5);
                    let pos = (
                        parts[1].parse().unwrap(),
                        parts[2].parse().unwrap(),
                        parts[3].parse().unwrap(),
                    );
                    scene.set_block(
                        pos,
                        (parts[4] != "minecraft:air").then(|| Block::new(parts[4])),
                    );
                }
            }
            "entity_set_random_seed" => {
                let seed = row["data"]["seed"].as_str().unwrap().parse().unwrap();
                match row["data"]["tag"].as_str().unwrap() {
                    "skeleton" => world
                        .skeleton_mut(skeleton_id.unwrap())
                        .unwrap()
                        .set_random_seed(seed),
                    "victim" if villager_id.is_some() => world
                        .villager_mut(villager_id.unwrap())
                        .unwrap()
                        .set_random_seed(seed),
                    "victim" if cow_id.is_some() => world
                        .cow_mut(cow_id.unwrap())
                        .unwrap()
                        .set_random_seed(seed as i64),
                    "victim" if zombie_id.is_some() => world
                        .zombie_mut(zombie_id.unwrap())
                        .unwrap()
                        .set_random_seed(seed),
                    "victim" if chicken_id.is_some() => world
                        .chicken_mut(chicken_id.unwrap())
                        .unwrap()
                        .set_random_seed(seed as i64),
                    tag => panic!("unsupported seed target: {tag}"),
                }
            }
            "projectile_shoot_seed" => world
                .set_arrow_shoot_seed(Some(row["data"]["seed"].as_str().unwrap().parse().unwrap())),
            "projectile_damage_seed" => world.set_arrow_damage_seed(Some(
                row["data"]["seed"].as_str().unwrap().parse().unwrap(),
            )),
            "entity_hurt" => {
                assert_eq!(row["data"]["tag"], "victim");
                assert_eq!(row["data"]["source"], "minecraft:generic");
                let victim = world.villager_mut(villager_id.unwrap()).unwrap();
                exact(
                    &row["data"]["health_before"],
                    f64::from(victim.villager.health),
                    row["tick"].as_i64().unwrap(),
                    "health before controlled hurt",
                );
                let result = victim.hurt(number(&row["data"]["amount"]) as f32);
                assert_eq!(row["data"]["applied"], result.applied);
                exact(
                    &row["data"]["health_after"],
                    f64::from(victim.villager.health),
                    row["tick"].as_i64().unwrap(),
                    "health after controlled hurt",
                );
            }
            "player_probe" => {
                let p = &row["data"]["pos"];
                players.push(PlayerCandidate {
                    id: 2,
                    position: DVec3::new(
                        p[0].as_f64().unwrap(),
                        p[1].as_f64().unwrap(),
                        p[2].as_f64().unwrap(),
                    ),
                    eye_height: 1.62,
                    main_hand_cow_food: false,
                    offhand_cow_food: false,
                    main_hand_pig_food: false,
                    offhand_pig_food: false,
                    main_hand_chicken_food: false,
                    offhand_chicken_food: false,
                    main_hand_carrot_on_a_stick: false,
                    offhand_carrot_on_a_stick: false,
                    main_hand_wolf_interest: false,
                    offhand_wolf_interest: false,
                    main_hand_horse_tempt: false,
                    offhand_horse_tempt: false,
                    alive: true,
                    spectator: false,
                    attackable: true,
                });
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = &data["entities"]["skeleton"][0];
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    for (key, block) in data["blocks"].as_object().unwrap() {
                        let pos: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        if block["id"] != "minecraft:air" {
                            scene.set_block(
                                (pos[0], pos[1], pos[2]),
                                Some(Block::new(block["id"].as_str().unwrap())),
                            );
                        }
                    }
                    let mut skeleton = Skeleton::new(DVec3::new(
                        number(&observed["x"]),
                        number(&observed["y"]),
                        number(&observed["z"]),
                    ));
                    skeleton.body.on_ground = observed["on_ground"].as_bool().unwrap();
                    skeleton.persistence_required =
                        observed["persistence_required"].as_bool().unwrap();
                    skeleton_id = Some(world.spawn_skeleton_bow(skeleton));
                    if let Some(victim) = data["entities"]["victim"]
                        .as_array()
                        .and_then(|rows| rows.first())
                    {
                        let position = DVec3::new(
                            number(&victim["x"]),
                            number(&victim["y"]),
                            number(&victim["z"]),
                        );
                        if victim["type"] == "minecraft:villager" {
                            let mut villager = Villager::new(position);
                            villager.body.on_ground = victim["on_ground"].as_bool().unwrap();
                            villager.persistence_required =
                                victim["persistence_required"].as_bool().unwrap();
                            villager_id = Some(world.spawn_villager(villager, true));
                        } else if victim["type"] == "minecraft:cow" {
                            let mut cow = Cow::new(position);
                            cow.body.on_ground = victim["on_ground"].as_bool().unwrap();
                            cow.persistence_required =
                                victim["persistence_required"].as_bool().unwrap();
                            cow_id = Some(world.spawn_cow(cow, true));
                        } else if victim["type"] == "minecraft:zombie" {
                            let mut zombie = Zombie::new(position);
                            zombie.body.on_ground = victim["on_ground"].as_bool().unwrap();
                            zombie.persistence_required =
                                victim["persistence_required"].as_bool().unwrap();
                            zombie_id = Some(world.spawn_zombie(zombie, true));
                        } else if victim["type"] == "minecraft:chicken" {
                            let mut chicken = Chicken::new(position);
                            chicken.body.on_ground = victim["on_ground"].as_bool().unwrap();
                            chicken.persistence_required =
                                victim["persistence_required"].as_bool().unwrap();
                            chicken_id = Some(world.spawn_chicken(chicken, true));
                        } else {
                            panic!("unsupported arrow victim: {}", victim["type"]);
                        }
                    }
                    continue;
                }
                frames += 1;
                world.tick_with_players(&mut scene, &players);
                assert_eq!(
                    data["game_time"],
                    world.game_time(),
                    "tick {tick} game time"
                );
                let entity = world
                    .skeletons()
                    .iter()
                    .find(|entity| Some(entity.id) == skeleton_id)
                    .unwrap();
                let body = &entity.skeleton.body;
                for (field, actual) in [
                    ("x", body.position.x),
                    ("y", body.position.y),
                    ("z", body.position.z),
                    ("vx", body.velocity.x),
                    ("vy", body.velocity.y),
                    ("vz", body.velocity.z),
                    ("yaw", f64::from(entity.yaw)),
                    ("speed", f64::from(entity.speed)),
                    ("head_yaw", f64::from(entity.look_control.head_yaw)),
                    ("body_yaw", f64::from(entity.body_rotation.body_yaw)),
                    ("pitch", f64::from(entity.look_control.pitch)),
                    ("move_control_x", entity.move_control.wanted.x),
                    ("move_control_y", entity.move_control.wanted.y),
                    ("move_control_z", entity.move_control.wanted.z),
                    ("look_control_x", entity.look_control.wanted.x),
                    ("look_control_y", entity.look_control.wanted.y),
                    ("look_control_z", entity.look_control.wanted.z),
                    (
                        "eye_y",
                        body.position.y + f64::from(entity.skeleton.eye_height()),
                    ),
                ] {
                    exact(&observed[field], actual, tick, field);
                }
                assert_eq!(observed["on_ground"], body.on_ground, "tick {tick} ground");
                assert_eq!(
                    observed["entity_tick_count"], entity.tick_count,
                    "tick {tick} count"
                );
                assert_eq!(
                    observed["ambient_sound_time"], entity.ambient_sound_time,
                    "tick {tick} ambient"
                );
                assert_eq!(
                    observed["no_action_time"], entity.no_action_time,
                    "tick {tick} inactivity"
                );
                assert_eq!(
                    observed["random_state"]
                        .as_str()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap(),
                    entity.random.raw_state(),
                    "tick {tick} random"
                );
                assert_eq!(
                    observed["using_item"], entity.bow.using_item,
                    "tick {tick} bow use"
                );
                assert_eq!(
                    observed["ticks_using_item"], entity.bow.ticks_using_item,
                    "tick {tick} bow ticks"
                );
                assert_eq!(
                    observed["aggressive"], entity.bow.aggressive,
                    "tick {tick} aggressive"
                );
                assert_eq!(observed["no_ai"], entity.no_ai, "tick {tick} no AI");
                exact(
                    &observed["health"],
                    f64::from(entity.skeleton.health),
                    tick,
                    "health",
                );
                assert_eq!(
                    observed["move_control_wanted"],
                    entity.move_control.has_wanted(),
                    "tick {tick} move wanted"
                );
                assert_eq!(
                    observed["look_control_wanted"],
                    entity.look_control.cooldown > 0,
                    "tick {tick} look wanted"
                );
                assert_eq!(
                    observed["path_node_count"],
                    entity.navigation.nodes.len(),
                    "tick {tick} path count"
                );
                assert_eq!(
                    observed["path_reached"], entity.navigation.reached,
                    "tick {tick} path reached"
                );
                assert_eq!(
                    observed["entity_numeric_id"], entity.id,
                    "tick {tick} numeric ID"
                );
                assert_eq!(
                    observed["use_item_id"],
                    if entity.bow.using_item {
                        "minecraft:bow"
                    } else {
                        "minecraft:air"
                    },
                    "tick {tick} used item"
                );
                assert_eq!(
                    observed["running_goals"],
                    serde_json::json!(["RangedBowAttackGoal"]),
                    "tick {tick} running goal"
                );
                assert!(observed.get("target_uuid").is_some(), "tick {tick} target");
                assert_eq!(
                    observed["navigation_done"],
                    entity.navigation.is_done(),
                    "tick {tick} nav done"
                );
                assert_eq!(
                    observed["path_next_node"],
                    entity.navigation.observed_next(),
                    "tick {tick} nav next"
                );
                let nodes: Vec<_> = entity
                    .navigation
                    .nodes
                    .iter()
                    .map(|node| vec![node.0, node.1, node.2])
                    .collect();
                assert_eq!(
                    observed["path_nodes"],
                    serde_json::to_value(nodes).unwrap(),
                    "tick {tick} path"
                );
                let arrows = data["entity_type_states"]["minecraft:arrow"]
                    .as_array()
                    .unwrap();
                assert_eq!(
                    arrows.len(),
                    world.arrows().len(),
                    "tick {tick} arrow count"
                );
                for (observed_arrow, arrow) in arrows.iter().zip(world.arrows()) {
                    assert_eq!(
                        observed_arrow["on_ground"], false,
                        "tick {tick} arrow ground"
                    );
                    if observed_arrow.get("arrow_in_ground").is_some() {
                        assert_eq!(observed_arrow["arrow_in_ground"], arrow.arrow.in_ground);
                        assert_eq!(
                            observed_arrow["arrow_in_ground_time"],
                            arrow.arrow.in_ground_time
                        );
                        assert_eq!(observed_arrow["arrow_life"], arrow.arrow.life);
                        assert_eq!(observed_arrow["arrow_shake_time"], arrow.arrow.shake_time);
                        if let Some(damage) = observed_arrow.get("arrow_base_damage") {
                            exact(damage, arrow.arrow.base_damage, tick, "arrow base damage");
                        }
                    }
                    for (field, actual) in [
                        ("x", arrow.arrow.position.x),
                        ("y", arrow.arrow.position.y),
                        ("z", arrow.arrow.position.z),
                        ("vx", arrow.arrow.velocity.x),
                        ("vy", arrow.arrow.velocity.y),
                        ("vz", arrow.arrow.velocity.z),
                    ] {
                        exact(&observed_arrow[field], actual, tick, field);
                    }
                }
                if let Some(id) = villager_id {
                    let reference = &data["entities"]["victim"][0];
                    let entity = world
                        .villagers()
                        .iter()
                        .find(|entity| entity.id == id)
                        .unwrap();
                    let body = &entity.villager.body;
                    for (field, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                        ("health", f64::from(entity.villager.health)),
                    ] {
                        exact(
                            &reference[field],
                            actual,
                            tick,
                            &format!("villager {field}"),
                        );
                    }
                    assert_eq!(reference["on_ground"], body.on_ground);
                    assert_eq!(reference["ambient_sound_time"], entity.ambient_sound_time);
                    assert_eq!(reference["entity_tick_count"], entity.tick_count);
                    assert_eq!(
                        reference["random_state"]
                            .as_str()
                            .unwrap()
                            .parse::<u64>()
                            .unwrap(),
                        entity.random.raw_state()
                    );
                }
                if let Some(id) = cow_id {
                    let reference = &data["entities"]["victim"][0];
                    let entity = world.cows().iter().find(|entity| entity.id == id).unwrap();
                    let body = &entity.cow.body;
                    for (field, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                        ("health", f64::from(entity.cow.health)),
                    ] {
                        exact(&reference[field], actual, tick, &format!("cow {field}"));
                    }
                    assert_eq!(reference["on_ground"], body.on_ground);
                    assert_eq!(reference["ambient_sound_time"], entity.ambient_sound_time);
                    assert_eq!(reference["entity_tick_count"], entity.tick_count);
                    assert_eq!(
                        reference["random_state"]
                            .as_str()
                            .unwrap()
                            .parse::<u64>()
                            .unwrap(),
                        entity.random.raw_state()
                    );
                }
                if let Some(id) = zombie_id {
                    let reference = &data["entities"]["victim"][0];
                    let entity = world
                        .zombies()
                        .iter()
                        .find(|entity| entity.id == id)
                        .unwrap();
                    let body = &entity.zombie.body;
                    for (field, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                        ("health", f64::from(entity.zombie.health)),
                    ] {
                        exact(&reference[field], actual, tick, &format!("zombie {field}"));
                    }
                    assert_eq!(reference["on_ground"], body.on_ground);
                    assert_eq!(reference["ambient_sound_time"], entity.ambient_sound_time);
                    assert_eq!(reference["entity_tick_count"], entity.tick_count);
                    assert_eq!(
                        reference["random_state"]
                            .as_str()
                            .unwrap()
                            .parse::<u64>()
                            .unwrap(),
                        entity.random.raw_state()
                    );
                }
                if let Some(id) = chicken_id {
                    let reference = &data["entities"]["victim"][0];
                    let entity = world
                        .chickens()
                        .iter()
                        .find(|entity| entity.id == id)
                        .unwrap();
                    let body = &entity.chicken.body;
                    for (field, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                        ("health", f64::from(entity.chicken.health)),
                    ] {
                        exact(&reference[field], actual, tick, &format!("chicken {field}"));
                    }
                    assert_eq!(reference["on_ground"], body.on_ground);
                    assert_eq!(reference["ambient_sound_time"], entity.ambient_sound_time);
                    assert_eq!(reference["entity_tick_count"], entity.tick_count);
                    assert_eq!(
                        reference["random_state"]
                            .as_str()
                            .unwrap()
                            .parse::<u64>()
                            .unwrap(),
                        entity.random.raw_state()
                    );
                }
            }
            "complete" => complete = true,
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(frames, expected_frames);
    println!("exact skeleton world bow trace: {frames} ticks");
}
