//! Shared by the mob gates: the sounds the mobs played in a tick against
//! the harness's `entity_sound_events` (event, position, volume and pitch
//! bits; the probe players' own sounds are the client's and left out), and
//! the block step sounds the gates' scenes know.
#![allow(dead_code)]
use minecraftoss_entities::world::EntityWorld;
use serde_json::Value;

/// A sound as the comparison sees it: event, position bits, volume bits,
/// pitch bits.
pub type Heard = (String, [u64; 3], u32, u32);

/// The x spans of a scenario's pens (its `fill`s): mobs left from earlier
/// scenarios keep sounding in theirs.
pub fn pens(suite: &Value, scenario: &str) -> Vec<(f64, f64)> {
    let Some(definition) = suite["scenarios"].as_array().and_then(|all| all.iter().find(|c| c["id"] == scenario)) else { return Vec::new() };
    definition["prepare"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|command| {
            let parts: Vec<&str> = command.as_str()?.split_whitespace().collect();
            (parts.first() == Some(&"fill")).then(|| (parts[1].parse::<f64>().unwrap(), parts[4].parse::<f64>().unwrap() + 1.0))
        })
        .collect()
}

/// Drains the world's sounds; where the snapshot recorded sounds they must
/// match vanilla's exactly (within the pens, when there are any). Returns
/// how many were compared.
pub fn check(world: &mut EntityWorld, data: &Value, at: &str, pens: &[(f64, f64)]) -> usize {
    check_heard(world, data, at, pens).0
}

/// [`check`], also handing back every sound the world made.
pub fn check_heard(world: &mut EntityWorld, data: &Value, at: &str, pens: &[(f64, f64)]) -> (usize, Vec<Heard>) {
    check_heard_with(world, &[], data, at, pens)
}

/// `check_heard` with the sounds blocks made (event, position; at volume and
/// pitch 1).
#[allow(dead_code)]
pub fn check_heard_with(world: &mut EntityWorld, blocks: &[(String, glam::DVec3)], data: &Value, at: &str, pens: &[(f64, f64)]) -> (usize, Vec<Heard>) {
    let mut ours: Vec<Heard> = world
        .take_sounds()
        .into_iter()
        .map(|s| (format!("minecraft:{}", s.event), [s.position.x.to_bits(), s.position.y.to_bits(), s.position.z.to_bits()], s.volume.to_bits(), s.pitch.to_bits()))
        .collect();
    ours.extend(blocks.iter().map(|(event, p)| (event.clone(), [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()], 1.0_f32.to_bits(), 1.0_f32.to_bits())));
    let Some(events) = data["entity_sound_events"].as_array() else { return (0, ours) };
    let bits = |v: &Value| u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap();
    let hex = |e: &Value, key: &str| u32::from_str_radix(e[key].as_str().unwrap(), 16).unwrap();
    let mut vanilla: Vec<Heard> = events
        .iter()
        .filter(|e| !e["id"].as_str().unwrap().starts_with("minecraft:entity.player."))
        .map(|e| {
            let p = e["position"].as_array().unwrap();
            (e["id"].as_str().unwrap().to_owned(), [bits(&p[0]), bits(&p[1]), bits(&p[2])], hex(e, "volume_bits"), hex(e, "pitch_bits"))
        })
        .filter(|s| {
            let x = f64::from_bits(s.1[0]);
            pens.is_empty() || pens.iter().any(|&(from, to)| x >= from && x < to)
        })
        .collect();
    ours.sort();
    vanilla.sort();
    // `MINECRAFTOSS_SOUNDS_LENIENT=1` reports a mismatch and carries on,
    // to reach the state comparison behind it.
    if std::env::var_os("MINECRAFTOSS_SOUNDS_LENIENT").is_some() && ours != vanilla {
        eprintln!("{at} sounds differ:
  ours    {ours:?}
  vanilla {vanilla:?}");
    } else {
        assert_eq!(ours, vanilla, "{at} sounds");
    }
    (vanilla.len(), ours)
}

/// The step sound of the blocks the fixtures build with (26.3 `Blocks`:
/// `SoundType.STONE` and `SoundType.GRASS`, volume and pitch 1); their
/// fall sounds share the family.
pub fn step_sound(id: &str) -> Option<(String, f32, f32)> {
    let family = match id {
        "minecraft:stone" => "stone",
        // `HayBlock` sounds as grass.
        "minecraft:grass_block" | "minecraft:hay_block" | "minecraft:sweet_berry_bush" => "grass",
        // Magma sounds as stone, fire and cactus as wool, campfires as wood.
        "minecraft:magma_block" => "stone",
        "minecraft:sand" => "sand",
        "minecraft:netherrack" => "netherrack",
        "minecraft:fire" | "minecraft:cactus" => "wool",
        "minecraft:campfire" => "wood",
        "minecraft:cobweb" => "cobweb",
        "minecraft:glass" => "glass",
        _ => return None,
    };
    Some((format!("block.{family}.step"), 1.0, 1.0))
}
