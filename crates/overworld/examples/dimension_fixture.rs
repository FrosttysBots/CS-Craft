//! Protected camera fixture, never used by gameplay.
use overworld::{
    terrain::{scene::Block, terrain::Dimension},
    *,
};
fn main() -> Result<(), String> {
    assert!(std::env::var_os("CSR_WORLD_DIR").is_some());
    let mut w = Runtime::load_seed(Some(732))?;
    let dim = if std::env::args().any(|s| s == "nether") {
        Dimension::Nether
    } else {
        Dimension::End
    };
    w.change_dimension(dim, None)?;
    let (x, y, z) = block_pos(w.save.player);
    for dx in -2i32..=2 {
        for dz in -2i32..=2 {
            for dy in -1..=3 {
                let edge = dx.abs() == 2 || dz.abs() == 2 || dy == -1 || dy == 3;
                w.set_block(
                    (x + dx, y + dy, z + dz),
                    if edge {
                        Some(Block::new("minecraft:obsidian"))
                    } else {
                        None
                    },
                );
            }
        }
    }
    w.save.survival.health = 20.;
    w.save_now()?;
    let origin = w.save.origin;
    let camera = to_source(
        origin,
        if dim == Dimension::End {
            [55., 95., 55.]
        } else {
            [
                w.save.player[0] + 5.,
                w.save.player[1] + 3.,
                w.save.player[2] + 5.,
            ]
        },
    );
    println!("CAMERA {} {} {} 17 135", camera.x, camera.y, camera.z);
    Ok(())
}
