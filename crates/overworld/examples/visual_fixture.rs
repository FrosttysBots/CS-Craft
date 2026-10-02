use overworld::{terrain::scene::Block, *};
fn main() -> Result<(), String> {
    assert!(std::env::var_os("CSR_WORLD_DIR").is_some());
    let mut w = Runtime::load_seed(Some(731))?;
    let (x, y, z) = block_pos(w.save.origin);
    for dx in -9..=12 {
        for dz in -9..=12 {
            for dy in -1..=8 {
                w.set_block(
                    (x + dx, y + dy, z + dz),
                    if dy == -1 {
                        Some(Block::new("minecraft:grass_block"))
                    } else {
                        None
                    },
                );
            }
        }
    }
    for dx in 2..=6 {
        for dz in 2..=6 {
            let mut b = Block::new("minecraft:red_mushroom_block");
            for face in ["up", "down", "north", "south", "east", "west"] {
                b.properties.insert(face.into(), "true".into());
            }
            w.set_block((x + dx, y + 3, z + dz), Some(b));
        }
    }
    for dy in 0..3 {
        w.set_block(
            (x + 4, y + dy, z + 4),
            Some(Block::new("minecraft:mushroom_stem")),
        );
    }
    w.set_block(
        (x + 1, y, z + 2),
        Some(Block::new("minecraft:crafting_table")),
    );
    w.set_block(
        (x + 2, y, z),
        Some(
            Block::new("minecraft:furnace")
                .with("facing", "west")
                .with("lit", "true"),
        ),
    );
    for (i, (id, n)) in [
        ("diamond_pickaxe", 1),
        ("oak_log", 16),
        ("cobblestone", 32),
        ("torch", 24),
        ("cooked_beef", 12),
        ("ender_eye", 12),
        ("obsidian", 14),
        ("flint_and_steel", 1),
        ("water_bucket", 1),
        ("diamond", 6),
        ("iron_ingot", 24),
        ("coal", 32),
    ]
    .into_iter()
    .enumerate()
    {
        w.inventory.slots[i] = Some(mc_player::inventory::ItemStack::new(
            format!("minecraft:{id}"),
            n,
        ));
    }
    w.save.player = [x as f64 + 0.5, y as f64 + 0.02, z as f64 + 0.5];
    w.save.day_ticks = 1000.;
    w.save_now()?;
    println!(
        "Fixture ready. origin={:?}, feet={:?}",
        w.save.origin,
        w.spawn_source()
    );
    Ok(())
}
