//! Real-asset checks for kill attribution, empty starts, mining and native models.
use mc_entities::{
    cow::Cow,
    world::{EntityWorld, MobHit, PlayerAttack},
    zombie::Zombie,
};
use mc_player::{
    inventory::{Inventory, ItemStack},
    mining::Mining,
};
use overworld::*;
use terrain::{
    mesh::{ChunkMesh, HeldDisplay, HeldItem, ItemVisuals},
    mob_actions::{Actor, attack, death_remains},
    pack::ResourceId,
    scene::{Block, HandcraftedScene},
};
fn main() -> Result<(), String> {
    let mut mobs = EntityWorld::default();
    let cow = mobs.spawn_cow(Cow::new(glam_mc::DVec3::ZERO), true);
    let zombie = mobs.spawn_zombie(Zombie::new(glam_mc::DVec3::new(5., 0., 0.)), true);
    let hit = PlayerAttack {
        player_id: 1,
        position: glam_mc::DVec3::ZERO,
        yaw: 0.,
        attack_damage: 100.,
        strength: 1.,
        sprinting: false,
        can_critical: false,
        can_sweep: false,
    };
    let mut inventory = Inventory::default();
    let mut actor = Actor {
        inventory: &mut inventory,
        selected: 0,
        infinite: false,
        entity_loot: None,
        shearing_loot: None,
    };
    let passive = attack(&mut mobs, MobHit::Cow(cow), &mut actor, &hit);
    let hostile = attack(&mut mobs, MobHit::Zombie(zombie), &mut actor, &hit);
    assert_eq!(passive.player_kills, vec!["minecraft:cow"]);
    assert_eq!(hostile.player_kills, vec!["minecraft:zombie"]);
    assert!(
        attack(&mut mobs, MobHit::Cow(cow), &mut actor, &hit)
            .player_kills
            .is_empty()
    );
    assert!(death_remains(&mut mobs, None).2.is_empty());
    let natural = mobs.spawn_cow(Cow::new(glam_mc::DVec3::ZERO), true);
    mobs.cow_mut(natural).unwrap().cow.hurt_generic(100.);
    assert!(death_remains(&mut mobs, None).2.is_empty());
    println!(
        "PASS: only player kills pay, passive $25 / hostile $100, no repeat or environmental payouts"
    );

    assert!(
        std::env::var_os("CSR_WORLD_DIR").is_some(),
        "Use an isolated test save"
    );
    let mut runtime = Runtime::load_seed(Some(853102))?;
    assert!(runtime.inventory.slots.iter().all(Option::is_none));
    runtime.save.survival.money = 0;
    for kind in passive.player_kills.iter().chain(&hostile.player_kills) {
        runtime.reward_mob(kind);
    }
    assert_eq!(runtime.save.survival.money, 125);
    assert_eq!(std::mem::take(&mut runtime.mob_kills), vec!["minecraft:cow", "minecraft:zombie"]);
    assert!(runtime.mob_kills.is_empty(), "feed records must be consumable once");
    runtime.save_now()?;
    let saved: Save =
        serde_json::from_slice(&std::fs::read(runtime.directory.join("world.json")).unwrap())
            .unwrap();
    assert_eq!(saved.survival.money, 125);
    let mut visuals = ItemVisuals::default();
    for id in [
        "diamond_pickaxe",
        "oak_log",
        "torch",
        "water_bucket",
        "cooked_beef",
        "ender_eye", "chest", "white_bed",
    ] {
        let mut mesh = ChunkMesh::default();
        visuals
            .append_held_items(
                &mut mesh,
                &[HeldItem {
                    id: format!("minecraft:{id}"),
                    pose: glam_mc::Mat4::IDENTITY,
                    light: glam_mc::Vec3::ZERO,
                    display: HeldDisplay::FirstPerson,
                    first_tint: None,
                }],
                &runtime.packs,
                &runtime.atlas,
                &runtime.light,
            )
            .map_err(|e| e.to_string())?;
        assert!(!mesh.indices.is_empty(), "Missing held model {id}");
        assert!(
            mesh.vertices
                .iter()
                .all(|v| v.position.iter().all(|v| v.is_finite()))
        );
    }
    for stage in 0..10 {
        assert!(runtime.atlas.contains(
            &ResourceId::parse(&format!("minecraft:block/destroy_stage_{stage}")).unwrap()
        ));
    }
    let mut miner = mc_player::Player::new(glam_mc::DVec3::new(0.5, 0., 0.5));
    miner.yaw = 0.;
    miner.on_ground = true;
    let mut scene = HandcraftedScene::default();
    scene.set((0, 1, 2), Some(Block::new("minecraft:stone")));
    let mut mining = Mining::default();
    for _ in 0..5 {
        mining.tick(&mut scene, &miner, None, true);
    }
    assert_eq!(mining.target(), Some((0, 1, 2)));
    assert!(mining.progress() > 0. && mining.progress() < 1.);
    assert!(mining.reset_if_item_changed(Some(&ItemStack::new("minecraft:diamond_pickaxe", 1))));
    assert_eq!(mining.progress(), 0.);
    mining.reset();
    assert_eq!(mining.target(), None);
    let mut a = ChunkMesh::default();
    let mut b = ChunkMesh::default();
    let mut dragon = adventure::Dragon::default();
    runtime.dragon_model.append(&mut a, &dragon, &runtime.atlas);
    dragon.phase_time = 0.25;
    runtime.dragon_model.append(&mut b, &dragon, &runtime.atlas);
    assert!(a.vertices.len() > 1000);
    assert_eq!(a.vertices.len(), b.vertices.len());
    assert!(
        a.vertices
            .iter()
            .zip(&b.vertices)
            .any(|(a, b)| a.position != b.position)
    );
    assert!(
        runtime
            .dragon_model
            .bounds(&dragon)
            .iter()
            .all(|(_, r)| r.iter().all(|v| v.is_finite() && *v > 0.))
    );
    const BOSS:u64=0xf000_0000_0000_0000;
    runtime.save.adventure.dimension="end".into();
    runtime.save.adventure.dragon=adventure::Dragon::default();
    assert!(runtime.hit_adventure(BOSS,5.));
    let mut hurt_dragon=ChunkMesh::default();runtime.append_adventure_mesh(&mut hurt_dragon,None);
    assert!(!hurt_dragon.vertices.is_empty());
    assert!(hurt_dragon.vertices.iter().all(|v| v.color[3]<0.),"whole dragon flashes red");
    assert_eq!(runtime.mob_hits.len(),1);
    runtime.mob_hits.clear();runtime.adventure_hurt.clear();
    runtime.save.adventure.dimension="nether".into();
    runtime.save.adventure.blazes=vec![adventure::Blaze{id:11,position:runtime.save.player,health:20.,cooldown:1.}];
    assert!(runtime.hit_adventure(BOSS+11,5.));
    let mut hurt_blaze=ChunkMesh::default();runtime.append_adventure_mesh(&mut hurt_blaze,None);
    assert!(!hurt_blaze.vertices.is_empty());
    assert!(hurt_blaze.vertices.iter().all(|v|v.color[3]<0.),"head and rods flash red");
    assert_eq!(runtime.mob_hits.len(),1);
    runtime.hit_adventure(BOSS+11,1000.);runtime.mob_hits.clear();
    runtime.hit_adventure(BOSS+11,1000.);assert!(runtime.mob_hits.is_empty(),"dead blaze does not bleed twice");
    runtime.hit_adventure(BOSS+1,50.);assert!(runtime.mob_hits.is_empty(),"crystals do not bleed");
    println!("PASS: native dragon and Blaze hurt meshes, accepted blood events, dead targets and crystal exclusion.");
    println!(
        "PASS: empty inventory, saved $125, eight pack-backed held models, ten crack stages, mining resets, native dragon geometry and animated poses; failures: 0"
    );
    Ok(())
}
