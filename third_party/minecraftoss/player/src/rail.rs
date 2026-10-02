//! RailState placement connections for the four vanilla rail blocks.
use crate::{Block, Pos, World};

pub fn is_rail(block: &Block) -> bool {
    matches!(
        block.id.as_str(),
        "minecraft:rail"
            | "minecraft:powered_rail"
            | "minecraft:detector_rail"
            | "minecraft:activator_rail"
    )
}

fn at(pos: Pos, delta: Pos) -> Pos {
    (pos.0 + delta.0, pos.1 + delta.1, pos.2 + delta.2)
}

fn rail_at(world: &impl World, pos: Pos) -> Option<(Pos, Block)> {
    for delta in [(0, 0, 0), (0, 1, 0), (0, -1, 0)] {
        let candidate = at(pos, delta);
        if let Some(block) = world.block(candidate).filter(is_rail) {
            return Some((candidate, block));
        }
    }
    None
}

fn connections(pos: Pos, shape: &str) -> Vec<Pos> {
    let deltas = match shape {
        "east_west" => [(-1, 0, 0), (1, 0, 0)],
        "ascending_east" => [(-1, 0, 0), (1, 1, 0)],
        "ascending_west" => [(-1, 1, 0), (1, 0, 0)],
        "ascending_north" => [(0, 1, -1), (0, 0, 1)],
        "ascending_south" => [(0, 0, -1), (0, 1, 1)],
        "south_east" => [(0, 0, 1), (1, 0, 0)],
        "south_west" => [(0, 0, 1), (-1, 0, 0)],
        "north_west" => [(0, 0, -1), (-1, 0, 0)],
        "north_east" => [(0, 0, -1), (1, 0, 0)],
        _ => [(0, 0, -1), (0, 0, 1)],
    };
    deltas.into_iter().map(|delta| at(pos, delta)).collect()
}

fn same_horizontal(a: Pos, b: Pos) -> bool {
    a.0 == b.0 && a.2 == b.2
}

fn soft_connections(world: &impl World, pos: Pos, block: &Block) -> Vec<Pos> {
    connections(pos, block.property("shape").unwrap_or("north_south"))
        .into_iter()
        .filter_map(|target| {
            let (rail_pos, rail) = rail_at(world, target)?;
            connections(rail_pos, rail.property("shape").unwrap_or("north_south"))
                .iter()
                .any(|&connection| same_horizontal(connection, pos))
                .then_some(rail_pos)
        })
        .collect()
}

fn can_connect(world: &impl World, candidate: Pos, rail: &Block, pos: Pos) -> bool {
    let connections = soft_connections(world, candidate, rail);
    connections.len() != 2 || connections.iter().any(|&c| same_horizontal(c, pos))
}

fn neighbor_rail(world: &impl World, pos: Pos, delta: Pos) -> bool {
    rail_at(world, at(pos, delta))
        .is_some_and(|(candidate, rail)| can_connect(world, candidate, &rail, pos))
}

pub fn potential_connections(world: &impl World, pos: Pos) -> usize {
    [(0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)]
        .into_iter()
        .filter(|&delta| rail_at(world, at(pos, delta)).is_some())
        .count()
}

fn slope_shape(world: &impl World, pos: Pos, shape: &'static str) -> &'static str {
    match shape {
        "north_south" => {
            let mut shape = shape;
            if world
                .block(at(pos, (0, 1, -1)))
                .is_some_and(|b| is_rail(&b))
            {
                shape = "ascending_north";
            }
            if world.block(at(pos, (0, 1, 1))).is_some_and(|b| is_rail(&b)) {
                shape = "ascending_south";
            }
            shape
        }
        "east_west" => {
            let mut shape = shape;
            if world.block(at(pos, (1, 1, 0))).is_some_and(|b| is_rail(&b)) {
                shape = "ascending_east";
            }
            if world
                .block(at(pos, (-1, 1, 0)))
                .is_some_and(|b| is_rail(&b))
            {
                shape = "ascending_west";
            }
            shape
        }
        _ => shape,
    }
}

fn connected_shape(world: &impl World, pos: Pos, links: &[Pos], straight: bool) -> &'static str {
    let n = links.iter().any(|&p| p.2 < pos.2);
    let s = links.iter().any(|&p| p.2 > pos.2);
    let w = links.iter().any(|&p| p.0 < pos.0);
    let e = links.iter().any(|&p| p.0 > pos.0);
    let mut shape = if w || e { "east_west" } else { "north_south" };
    if !straight {
        shape = match (n, s, w, e) {
            (false, true, false, true) => "south_east",
            (false, true, true, false) => "south_west",
            (true, false, true, false) => "north_west",
            (true, false, false, true) => "north_east",
            _ => shape,
        };
    }
    slope_shape(world, pos, shape)
}

/// Reproduce `BaseRailBlock.updateDir`/`RailState.place` for a newly placed
/// rail. Removal is intentionally separate: an ordinary rail can retain its
/// corner shape until a later rail placement reconnects it.
pub fn placed(world: &mut impl World, pos: Pos, has_signal: bool) -> Vec<Pos> {
    update_dir(world, pos, has_signal, true)
}

/// RailBlock.updateState only reaches this path for a signal-source neighbor
/// when the ordinary rail has exactly three potential rail connections.
pub fn signal_neighbor_changed(world: &mut impl World, pos: Pos, has_signal: bool) -> Vec<Pos> {
    if potential_connections(world, pos) != 3 {
        return Vec::new();
    }
    update_dir(world, pos, has_signal, false)
}

fn update_dir(world: &mut impl World, pos: Pos, has_signal: bool, first: bool) -> Vec<Pos> {
    let Some(mut block) = world.block(pos).filter(is_rail) else {
        return Vec::new();
    };
    let straight = block.id != "minecraft:rail";
    let default = block.property("shape").unwrap_or("north_south").to_owned();
    let n = neighbor_rail(world, pos, (0, 0, -1));
    let s = neighbor_rail(world, pos, (0, 0, 1));
    let w = neighbor_rail(world, pos, (-1, 0, 0));
    let e = neighbor_rail(world, pos, (1, 0, 0));
    let ns = n || s;
    let ew = w || e;
    let mut shape = None;
    if ns && !ew {
        shape = Some("north_south");
    }
    if ew && !ns {
        shape = Some("east_west");
    }
    if !straight {
        shape = match (n, s, w, e) {
            (false, true, false, true) => Some("south_east"),
            (false, true, true, false) => Some("south_west"),
            (true, false, true, false) => Some("north_west"),
            (true, false, false, true) => Some("north_east"),
            _ => shape,
        };
    }
    if shape.is_none() {
        shape = if ns && ew {
            Some(default.as_str())
        } else if ns {
            Some("north_south")
        } else if ew {
            Some("east_west")
        } else {
            None
        };
        if !straight {
            let corners = if has_signal {
                [
                    (s && e, "south_east"),
                    (s && w, "south_west"),
                    (n && e, "north_east"),
                    (n && w, "north_west"),
                ]
            } else {
                [
                    (n && w, "north_west"),
                    (n && e, "north_east"),
                    (s && w, "south_west"),
                    (s && e, "south_east"),
                ]
            };
            for (matches, corner) in corners {
                if matches {
                    shape = Some(corner);
                }
            }
        }
    }
    let shape = shape.unwrap_or(&default);
    let shape = slope_shape(
        world,
        pos,
        match shape {
            "north_south" => "north_south",
            "east_west" => "east_west",
            "north_east" => "north_east",
            "north_west" => "north_west",
            "south_east" => "south_east",
            "south_west" => "south_west",
            "ascending_north" => "ascending_north",
            "ascending_south" => "ascending_south",
            "ascending_east" => "ascending_east",
            "ascending_west" => "ascending_west",
            _ => "north_south",
        },
    );
    if !first && block.property("shape") == Some(shape) {
        return Vec::new();
    }
    block.properties.insert("shape".into(), shape.into());
    world.set_block(pos, Some(block));
    let mut changed = vec![pos];
    for link in connections(pos, shape) {
        let Some((neighbor_pos, mut neighbor)) = rail_at(world, link) else {
            continue;
        };
        let mut links = soft_connections(world, neighbor_pos, &neighbor);
        if links.len() == 2 && !links.iter().any(|&p| same_horizontal(p, pos)) {
            continue;
        }
        links.push(pos);
        let next = connected_shape(world, neighbor_pos, &links, neighbor.id != "minecraft:rail");
        if neighbor.property("shape") != Some(next) {
            neighbor.properties.insert("shape".into(), next.into());
            world.set_block(neighbor_pos, Some(neighbor));
            changed.push(neighbor_pos);
        }
    }
    changed
}
