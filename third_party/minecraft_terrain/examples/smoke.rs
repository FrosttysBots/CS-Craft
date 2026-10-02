//! Streams a seeded world around its spawn for a few seconds and reports what
//! the section mesher produced. `MINECRAFTOSS_ROOT` names the MinecraftOSS
//! checkout; the resource pack is its local vanilla pack.
use std::sync::Arc;
use std::time::Instant;

use minecraft_terrain::{mesh, pack::PackStack, scene::HandcraftedScene, sections::CullCamera, terrain};
use minecraftoss_core::registries::{DataPaths, Registries};

fn main() -> anyhow::Result<()> {
    let seed: i64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(42);
    let root = std::path::PathBuf::from(std::env::var("MINECRAFTOSS_ROOT")?);
    let started = Instant::now();
    let paths = DataPaths::discover().map_err(anyhow::Error::msg)?;
    let registries = Arc::new(Registries::load(&paths).map_err(anyhow::Error::msg)?);
    println!("registries {:.2}s", started.elapsed().as_secs_f64());
    let packs = PackStack::open(vec![root.join("resourcepacks/local/minecraft-26.3")])?;
    let started = Instant::now();
    let mut stream = terrain::TerrainStream::for_dimension(
        registries,
        seed,
        6,
        terrain::Dimension::Overworld,
        None,
    )?;
    println!(
        "generator and spawn {:.2}s spawn={:?} player={:?}",
        started.elapsed().as_secs_f64(),
        stream.world_spawn,
        stream.player_spawn
    );
    let started = Instant::now();
    let build = mesh::build(&HandcraftedScene::default(), &packs)?;
    println!(
        "atlas {:.2}s {}x{}",
        started.elapsed().as_secs_f64(),
        build.atlas.pixels.width(),
        build.atlas.pixels.height()
    );
    let mut scene = HandcraftedScene::streamed(stream.states.clone());
    let (px, py, pz) = stream.player_spawn;
    let block = (px.floor() as i32, py.floor() as i32, pz.floor() as i32);
    let camera = CullCamera {
        position: glam::DVec3::new(px, py + 1.6, pz),
        forward: glam::Vec3::new(0.0, 0.0, -1.0),
        fov_degrees: 70.0,
        aspect: 16.0 / 9.0,
        yaw_degrees: 0.0,
        pitch_degrees: 0.0,
    };
    let (mut sections, mut vertices) = (0usize, 0usize);
    let started = Instant::now();
    while started.elapsed().as_secs_f64() < 5.0 {
        stream.server_tick(block, &mut scene);
        let update = stream.frame(&scene, &camera, 0, &build.atlas, &packs);
        for (_, mesh) in &update.uploads {
            sections += 1;
            vertices += mesh.vertices.len();
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    println!("5s: {sections} section uploads, {vertices} vertices");
    if let Some((pos, mesh)) = stream_first(&mut stream, &mut scene, block, &camera, &build, &packs) {
        println!("sample section {pos:?}: first vertex {:?}", mesh.vertices.first());
    }
    Ok(())
}

fn stream_first(
    stream: &mut terrain::TerrainStream,
    scene: &mut HandcraftedScene,
    block: (i32, i32, i32),
    camera: &CullCamera,
    build: &mesh::Build,
    packs: &PackStack,
) -> Option<(minecraft_terrain::sections::SectionPos, mesh::SectionMesh)> {
    stream.remesh_all(scene);
    for _ in 0..200 {
        stream.server_tick(block, scene);
        let update = stream.frame(scene, camera, 0, &build.atlas, packs);
        if let Some(first) = update.uploads.into_iter().find(|(_, m)| !m.vertices.is_empty()) {
            return Some(first);
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    None
}
