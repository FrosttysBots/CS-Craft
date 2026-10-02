//! External-asset regression: resource gates, recipes, furnace, geometry, dimensions and victory.
use collision::DynamicWorld;
use overworld::{
    mc_player::{
        furnace::Furnace,
        inventory::{Inventory, ItemStack},
        loot::LootBook,
        mining::Mining,
    },
    terrain::{
        pack::PackStack,
        scene::{Block, HandcraftedScene},
        terrain::Dimension,
    },
    *,
};
use std::sync::Arc;
fn stack(id: &str, n: u8) -> Option<ItemStack> {
    Some(ItemStack::new(format!("minecraft:{id}"), n))
}
fn recipe(inv: &mut Inventory, ids: &[&str], size: usize, expected: &str) {
    let grid: Vec<_> = ids
        .iter()
        .map(|id| if id.is_empty() { None } else { stack(id, 1) })
        .collect();
    let output = inv
        .recipes
        .matching(&grid, size, size)
        .unwrap_or_else(|| panic!("missing recipe: {ids:?}"));
    assert_eq!(output.id, format!("minecraft:{expected}"));
    if size == 2 {
        inv.crafting = grid.try_into().unwrap();
        assert!(inv.take_crafting_output(true));
        assert!(inv.crafting.iter().all(Option::is_none));
    } else {
        inv.workbench = grid.try_into().unwrap();
        assert!(inv.take_workbench_output(true));
        assert!(inv.workbench.iter().all(Option::is_none));
    }
}
fn main() -> Result<(), String> {
    let root = asset_root()?;
    let jar = root.join("client.jar");
    let book = Arc::new(survival::recipe_book(&root).map_err(|e| e.to_string())?);
    let mut inv = Inventory::default();
    inv.recipes = book.clone();
    assert_eq!(
        book.durability(&ItemStack::new("minecraft:diamond_pickaxe", 1)),
        Some((0, 1561))
    );
    recipe(&mut inv, &["oak_log", "", "", ""], 2, "oak_planks");
    recipe(
        &mut inv,
        &["oak_planks", "oak_planks", "oak_planks", "oak_planks"],
        2,
        "crafting_table",
    );
    recipe(&mut inv, &["oak_planks", "", "oak_planks", ""], 2, "stick");
    for (material, expected) in [
        ("oak_planks", "wooden_pickaxe"),
        ("cobblestone", "stone_pickaxe"),
        ("iron_ingot", "iron_pickaxe"),
        ("diamond", "diamond_pickaxe"),
    ] {
        recipe(
            &mut inv,
            &[
                material, material, material, "", "stick", "", "", "stick", "",
            ],
            3,
            expected,
        );
    }
    recipe(
        &mut inv,
        &[
            "cobblestone",
            "cobblestone",
            "cobblestone",
            "cobblestone",
            "",
            "cobblestone",
            "cobblestone",
            "cobblestone",
            "cobblestone",
        ],
        3,
        "furnace",
    );
    recipe(
        &mut inv,
        &["iron_ingot", "", "", "flint"],
        2,
        "flint_and_steel",
    );
    recipe(&mut inv, &["blaze_rod", "", "", ""], 2, "blaze_powder");
    recipe(
        &mut inv,
        &["ender_pearl", "blaze_powder", "", ""],
        2,
        "ender_eye",
    );
    let mut f = Furnace::default();
    f.slots[0] = stack("raw_iron", 3);
    f.slots[1] = stack("coal", 1);
    for _ in 0..600 {
        f.tick(&book);
    }
    assert_eq!(
        f.slots[2].as_ref().map(|s| (&*s.id, s.count)),
        Some(("minecraft:iron_ingot", 3))
    );
    for (block, tool, expected) in [
        ("oak_log", None, Some("oak_log")),
        ("stone", None, None),
        ("stone", Some("wooden_pickaxe"), Some("cobblestone")),
        ("iron_ore", Some("stone_pickaxe"), Some("raw_iron")),
        ("diamond_ore", Some("iron_pickaxe"), Some("diamond")),
        ("obsidian", Some("iron_pickaxe"), None),
        ("obsidian", Some("diamond_pickaxe"), Some("obsidian")),
    ] {
        let mut scene = HandcraftedScene::default();
        scene.set((0, 1, 2), Some(Block::new(&format!("minecraft:{block}"))));
        let mut p = mc_player::Player::new(glam_mc::DVec3::new(0.5, 0., 0.5));
        p.on_ground = true;
        p.yaw = 0.;
        let mut mining = Mining::default();
        mining.set_loot(LootBook::from_jar(&jar).map_err(|e| e.to_string())?);
        let held = tool.and_then(|s| stack(s, 1));
        let mut broken = None;
        for _ in 0..10000 {
            if let Some(b) = mining.tick(&mut scene, &p, held.as_ref(), true) {
                broken = Some(b);
                break;
            }
        }
        let b = broken.unwrap_or_else(|| panic!("could not mine {block}"));
        assert!(!b.loot_unresolved, "unresolved loot {block}");
        if let Some(expected) = expected {
            assert!(
                b.drops
                    .iter()
                    .any(|s| s.id == format!("minecraft:{expected}")),
                "{block}: {:?}",
                b.drops
            );
        } else {
            assert!(b.drops.is_empty(), "wrong tool must not harvest {block}");
        }
    }
    let packs = PackStack::open(vec![root.join("resourcepacks/local/minecraft-26.3")])
        .map_err(|e| e.to_string())?;
    let mut scene = HandcraftedScene::default();
    let mut b = Block::new("minecraft:red_mushroom_block");
    for face in ["up", "down", "north", "south", "east", "west"] {
        b.properties.insert(face.into(), "true".into());
    }
    scene.set((0, 0, 0), Some(b));
    let mesh = terrain::mesh::build_minimal(&scene, &packs).map_err(|e| e.to_string())?;
    // The six cap faces must land on six different planes (formerly X rotations overlapped north).
    let vertices = &mesh.chunks.values().next().unwrap().vertices;
    let mut planes = std::collections::HashSet::new();
    for q in vertices.chunks_exact(4) {
        for axis in 0..3 {
            let c = q[0].position[axis];
            if q.iter().all(|v| (v.position[axis] - c).abs() < 0.001) {
                planes.insert((axis, (c * 1000.).round() as i32));
            }
        }
    }
    assert!(planes.len() >= 6, "mushroom faces: {planes:?}");
    println!(
        "PASS: all progression recipes, furnace fuel/cooking, tool-gated loot, six mushroom faces"
    );
    if std::env::var_os("CSR_TEST_DIMENSIONS").is_none() {
        return Ok(());
    }
    assert!(
        std::env::var_os("CSR_WORLD_DIR").is_some(),
        "use an isolated test save"
    );
    let mut w = Runtime::load_seed(Some(94217))?;
    assert!(
        w.inventory.slots.iter().all(Option::is_none),
        "new Survival inventory starts empty"
    );
    assert_ne!(
        w.atlas.region(
            &terrain::pack::ResourceId::parse("minecraft:entity/end_portal/end_portal").unwrap()
        ),
        w.atlas
            .region(&terrain::pack::ResourceId::parse("minecraft:missingno").unwrap())
    );
    let home = w.save.player;
    w.screen = survival::Screen::Inventory;
    w.inventory.slots[0] = stack("oak_log", 7);
    w.click_slot(survival::Slot::Inventory(0), true, false);
    assert_eq!(w.inventory.cursor.as_ref().unwrap().count, 4);
    w.click_slot(survival::Slot::Craft(0), true, false);
    assert_eq!(w.inventory.crafting[0].as_ref().unwrap().count, 1);
    assert_eq!(
        w.inventory.crafting_output().unwrap().id,
        "minecraft:oak_planks"
    );
    w.close_screen();
    assert_eq!(w.inventory.count("minecraft:oak_log"), 7);
    let mut scene = HandcraftedScene::default();
    scene.set((0, 1, 2), Some(Block::new("minecraft:obsidian")));
    let mut miner = mc_player::Player::new(glam_mc::DVec3::new(0.5, 0., 0.5));
    miner.yaw = 0.;
    miner.on_ground = true;
    let held = ItemStack::new("minecraft:iron_pickaxe", 1);
    let mut result = None;
    for _ in 0..10000 {
        if let Some(b) = w.mining.tick(&mut scene, &miner, Some(&held), true) {
            result = Some(b);
            break;
        }
    }
    assert!(
        result
            .expect("registry-backed obsidian mining")
            .drops
            .is_empty()
    );
    println!("PASS: inventory split/place/close conserves items; registry-backed tool gates");
    w.inventory = inv;
    w.save
        .survival
        .furnaces
        .insert("overworld:0,64,0".into(), f);
    w.change_dimension(Dimension::Nether, None)?;
    assert_eq!(w.save.adventure.dimension, "nether");
    assert!(w.inventory.count("minecraft:diamond_pickaxe") > 0);
    // Exercise the custom fortress spawn and the actual dropped-item pickup path.
    let (px, py, pz) = block_pos(w.save.player);
    for dx in 10..=14 {
        for dz in -2..=2 {
            for dy in -1..=4 {
                assert!(w.set_block(
                    (px + dx, py + dy, pz + dz),
                    if dy == -1 {
                        Some(Block::new("minecraft:nether_bricks"))
                    } else {
                        None
                    }
                ));
            }
        }
    }
    w.save.day_ticks = 100.;
    w.tick_adventure(0.05, true);
    let blaze = w
        .save
        .adventure
        .blazes
        .first()
        .expect("fortress blaze spawned")
        .clone();
    let before = w.inventory.count("minecraft:blaze_rod");
    assert!(w.hit_adventure(0xf000_0000_0000_0000 + blaze.id, 100.));
    let pickup = to_source(w.save.origin, blaze.position);
    for _ in 0..80 {
        w.tick(0.05, pickup, true);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        w.inventory.count("minecraft:blaze_rod") > before,
        "blaze rod must be collectable"
    );
    println!("PASS: fortress blaze spawn, damage, rod drop and collection");
    w.change_dimension(Dimension::Overworld, Some(home))?;
    let stronghold = w.locate_stronghold().expect("generated stronghold ring");
    println!("PASS: Nether roundtrip, persistent inventory, stronghold at {stronghold:?}");
    w.change_dimension(Dimension::End, None)?;
    let spawn = w.spawn_source();
    assert!(
        !w.collision
            .trace_box(
                spawn,
                spawn,
                glam::Vec3::new(-16., -16., 0.),
                glam::Vec3::new(16., 16., 72.),
                collision::contents::MASK_PLAYERSOLID
            )
            .start_solid,
        "End arrival must clear every crystal pillar"
    );

    w.tick_adventure(0., true);
    let center = to_source(w.save.origin, w.dragon_model.bounds(&w.save.adventure.dragon)[0].0);
    let from = center + glam::Vec3::Z * BLOCK * 12.;
    let (target, _) = w
        .collision
        .hit_target(from, center)
        .expect("rendered dragon body has a shootable target");
    assert_eq!(target, 0xf000_0000_0000_0000);
    w.collision.damage_target(target, 40., from);
    w.tick(0.05, spawn, true);
    assert!(
        w.save.adventure.dragon.health < 200.,
        "CS damage reaches the dragon through the collision bridge"
    );
    println!("PASS: CS ray target and queued damage bridge hit the rendered dragon body");
    assert!(w.save.adventure.dragon.crystals.iter().all(|c| c.alive));
    assert_eq!(w.save.adventure.dragon.crystals.len(), 10);
    for i in 1..=10 {
        w.hit_adventure(0xf000_0000_0000_0000 + i, 40.);
    }
    for _ in 0..100 {
        w.hit_adventure(0xf000_0000_0000_0000, 40.);
    }
    for _ in 0..220 {
        w.tick_adventure(0.05, true);
    }
    assert!(w.save.adventure.defeated);
    assert_eq!(w.block_name((1, 64, 0)).as_deref(), Some("end_portal"));
    w.save.player = [1.5, 64., 0.5];
    w.portal_cooldown = 0.;
    for _ in 0..5 {
        w.tick_adventure(0.05, true);
    }
    assert!(w.save.adventure.completed);
    w.save_now()?;
    drop(w);
    let w = Runtime::load_seed(Some(94217))?;
    assert!(w.save.adventure.completed);
    assert!(w.inventory.count("minecraft:diamond_pickaxe") > 0);
    assert_eq!(w.save.adventure.dimension, "end");
    println!(
        "PASS: End crystals, dragon damage/death, exit portal, completion and save/reload; failures: 0"
    );
    Ok(())
}
