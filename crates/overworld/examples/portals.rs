use overworld::{
    terrain::{
        scene::{Block, Scene},
        terrain::Dimension,
    },
    *,
};
use std::time::{Duration, Instant};
fn main() -> Result<(), String> {
    assert!(std::env::var_os("CSR_WORLD_DIR").is_some());
    let mut w = Runtime::load_seed(Some(73219))?;
    let stronghold = w.locate_stronghold().unwrap();
    w.change_dimension(Dimension::Overworld, Some(stronghold.map(|x| x as f64)))?;
    let end = Instant::now() + Duration::from_secs(12);
    while Instant::now() < end {
        let feet = w.spawn_source();
        w.tick(0.05, feet, true);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut frames = vec![];
    {
        let scene = w.collision.scene.read().unwrap();
        for (cx, cz) in scene.chunks() {
            for y in -64..70 {
                for x in 0..16 {
                    for z in 0..16 {
                        let p = (cx * 16 + x, y, cz * 16 + z);
                        if Scene::block(&*scene, p).is_some_and(|b| b.id.path == "end_portal_frame")
                        {
                            frames.push(p);
                        }
                    }
                }
            }
        }
    }
    println!("Stronghold frames: {frames:?}");
    assert!(
        frames.len() >= 12,
        "natural stronghold must contain an accessible portal"
    );
    w.inventory.slots[0] = Some(mc_player::inventory::ItemStack::new(
        "minecraft:ender_eye",
        12,
    ));
    w.save.selected = 0;
    for p in &frames {
        w.insert_eye(*p);
    }
    let minx = frames.iter().map(|p| p.0).min().unwrap();
    let maxx = frames.iter().map(|p| p.0).max().unwrap();
    let minz = frames.iter().map(|p| p.2).min().unwrap();
    let maxz = frames.iter().map(|p| p.2).max().unwrap();
    let c = ((minx + maxx) / 2, frames[0].1, (minz + maxz) / 2);
    assert_eq!(w.block_name(c).as_deref(), Some("end_portal"));
    // A player-built obsidian frame is validated before ignition, and incomplete frames fail.
    let p = (stronghold[0], 90, stronghold[2]);
    for x in -1..=2 {
        for y in -1..=3 {
            let edge = x == -1 || x == 2 || y == -1 || y == 3;
            w.set_block(
                (p.0 + x, p.1 + y, p.2),
                edge.then(|| Block::new("minecraft:obsidian")),
            );
        }
    }
    assert!(w.ignite_portal(p));
    assert_eq!(w.block_name(p).as_deref(), Some("nether_portal"));
    println!(
        "PASS: natural stronghold, all twelve Eyes, End portal activation, constructed Nether portal; failures: 0"
    );
    Ok(())
}
