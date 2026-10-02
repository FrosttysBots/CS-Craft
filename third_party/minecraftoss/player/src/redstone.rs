//! Initial 26.3 redstone block-update slice. The reference observations for the
//! supported lever/dust/lamp circuit are in scenarios/redstone-lever-lamp.json.
use crate::{daylight::detector_power, ticks::ScheduledTicks, Block, Pos, World};
use std::collections::{HashMap, HashSet, VecDeque};

const HORIZONTAL: [(i32, i32, i32, &str); 4] = [
    (0, 0, -1, "north"),
    (1, 0, 0, "east"),
    (0, 0, 1, "south"),
    (-1, 0, 0, "west"),
];
const OBSERVER_DIRECTIONS: [(i32, i32, i32, &str); 6] = [
    (0, 0, -1, "north"),
    (1, 0, 0, "east"),
    (0, 0, 1, "south"),
    (-1, 0, 0, "west"),
    (0, 1, 0, "up"),
    (0, -1, 0, "down"),
];
const ROD_IDS: [&str; 8] = [
    "minecraft:lightning_rod",
    "minecraft:exposed_lightning_rod",
    "minecraft:weathered_lightning_rod",
    "minecraft:oxidized_lightning_rod",
    "minecraft:waxed_lightning_rod",
    "minecraft:waxed_exposed_lightning_rod",
    "minecraft:waxed_weathered_lightning_rod",
    "minecraft:waxed_oxidized_lightning_rod",
];

fn rod_tick_kind(id: &str) -> Option<TickKind> {
    ROD_IDS
        .iter()
        .position(|&candidate| candidate == id)
        .map(|index| TickKind::LightningRod(index as u8))
}

fn is_lightning_rod(id: &str) -> bool {
    rod_tick_kind(id).is_some()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlateMaterial {
    Wooden,
    Stone,
    Metal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlateKind {
    Wooden,
    Stone,
    LightWeighted,
    HeavyWeighted,
}

impl PlateKind {
    fn material(self) -> PlateMaterial {
        match self {
            Self::Wooden => PlateMaterial::Wooden,
            Self::Stone => PlateMaterial::Stone,
            Self::LightWeighted | Self::HeavyWeighted => PlateMaterial::Metal,
        }
    }
    fn state_signal(self, block: &Block) -> u8 {
        match self {
            Self::LightWeighted | Self::HeavyWeighted => block
                .property("power")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            _ => u8::from(block.property("powered") == Some("true")) * 15,
        }
    }
    fn contact_signal(self, contacts: (u16, u16)) -> u8 {
        match self {
            Self::Wooden => u8::from(contacts.0 > 0) * 15,
            Self::Stone => u8::from(contacts.1 > 0) * 15,
            Self::LightWeighted => ((contacts.0.min(15) as f32 / 15.0) * 15.0).ceil() as u8,
            Self::HeavyWeighted => ((contacts.0.min(150) as f32 / 150.0) * 15.0).ceil() as u8,
        }
    }
    fn pressed_time(self) -> u64 {
        match self {
            Self::LightWeighted | Self::HeavyWeighted => 10,
            _ => 20,
        }
    }
}

fn pressure_plate_kind(block: &Block) -> Option<PlateKind> {
    match block.id.as_str() {
        "minecraft:light_weighted_pressure_plate" => Some(PlateKind::LightWeighted),
        "minecraft:heavy_weighted_pressure_plate" => Some(PlateKind::HeavyWeighted),
        "minecraft:stone_pressure_plate" | "minecraft:polished_blackstone_pressure_plate" => {
            Some(PlateKind::Stone)
        }
        id if id.starts_with("minecraft:") && id.ends_with("_pressure_plate") => {
            Some(PlateKind::Wooden)
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum TickKind {
    Lamp,
    PressurePlate,
    LightningRod(u8),
    Button,
    Torch,
    TorchRestart,
    Repeater,
    Comparator,
    Observer,
    /// true for dropper, false for dispenser; the scheduled block identity
    /// survives even if the state at this position is replaced meanwhile.
    Dispenser(bool),
    PistonExtendFinish,
    PistonRetractFinish,
    PistonPayloadFinish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UseSound {
    None,
    Lever {
        powered: bool,
    },
    StoneButton {
        powered: bool,
    },
    WoodButton {
        powered: bool,
    },
    OakTrapdoor {
        open: bool,
    },
    OakFenceGate {
        open: bool,
    },
    OakDoor {
        open: bool,
    },
    IronDoor {
        open: bool,
    },
    Comparator {
        subtract: bool,
    },
    CopperBulb {
        lit: bool,
    },
    Piston {
        extending: bool,
    },
    PressurePlate {
        material: PlateMaterial,
        powered: bool,
    },
}

/// World-space entity bounds used by the source plate's 14×14×4/16 touch box.
#[derive(Clone, Copy, Debug)]
pub struct PlateEntity {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub living: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteEvent {
    pub pos: Pos,
    pub instrument: String,
    pub note: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PistonEvent {
    pub pos: Pos,
    pub id: u8,
    pub param: u8,
}

/// A scheduled dispenser/dropper activation. The block entity chooses a slot
/// and applies its dispense behavior after this block tick, not on the edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispenseEvent {
    pub pos: Pos,
    pub block_id: String,
    pub facing: String,
}

#[derive(Clone, Debug)]
pub struct PistonVisual {
    pub pos: Pos,
    pub facing: String,
    pub moved_state: Block,
    pub extending: bool,
    pub source: bool,
    pub began_at: u64,
}

#[derive(Clone, Debug)]
struct PistonMotion {
    facing: String,
    extending: bool,
    sticky: bool,
}

#[derive(Clone, Default)]
pub struct RedstoneEngine {
    ticks: ScheduledTicks<TickKind>,
    sounds: Vec<(Pos, UseSound)>,
    torch_toggles: VecDeque<(Pos, u64)>,
    conductor_power: HashMap<Pos, u8>,
    comparator_output: HashMap<Pos, u8>,
    container_signal: HashMap<Pos, u8>,
    executed_ticks: Vec<(Pos, String)>,
    note_events: Vec<NoteEvent>,
    piston_events: Vec<PistonEvent>,
    dispense_events: Vec<DispenseEvent>,
    piston_motion: HashMap<Pos, PistonMotion>,
    piston_payloads: HashMap<Pos, Block>,
    piston_visuals: HashMap<Pos, PistonVisual>,
    plate_contacts: HashMap<Pos, (u16, u16)>,
}

impl RedstoneEngine {
    /// BaseRailBlock.updateDir after a rail enters the world.
    pub fn rail_placed(&mut self, world: &mut impl World, pos: Pos) -> Vec<Pos> {
        let has_signal = has_neighbor_signal(world, pos, &self.comparator_output);
        crate::rail::placed(world, pos, has_signal)
    }

    /// Called with the currently intersecting entities before scheduled ticks
    /// and again after movement. Empty plates release only at their 20-tick
    /// scheduled check, as in BasePressurePlateBlock.tick.
    pub fn update_pressure_plate_entities(
        &mut self,
        world: &mut impl World,
        entities: &[PlateEntity],
        now: u64,
    ) -> Vec<Pos> {
        let mut contacts = HashMap::<Pos, (u16, u16)>::new();
        let mut plate_positions = Vec::new();
        for entity in entities {
            let lo = entity.min.map(|value| value.floor() as i32);
            let hi = entity.max.map(|value| value.floor() as i32);
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        let pos = (x, y, z);
                        if !world
                            .block(pos)
                            .is_some_and(|b| pressure_plate_kind(&b).is_some())
                        {
                            continue;
                        }
                        let plate_min = [x as f64 + 0.0625, y as f64, z as f64 + 0.0625];
                        let plate_max = [x as f64 + 0.9375, y as f64 + 0.25, z as f64 + 0.9375];
                        if (0..3).all(|axis| {
                            entity.max[axis] > plate_min[axis] && entity.min[axis] < plate_max[axis]
                        }) {
                            if !contacts.contains_key(&pos) {
                                plate_positions.push(pos);
                            }
                            let contact = contacts.entry(pos).or_default();
                            contact.0 = contact.0.saturating_add(1);
                            if entity.living {
                                contact.1 = contact.1.saturating_add(1);
                            }
                        }
                    }
                }
            }
        }
        self.plate_contacts = contacts;
        let mut changed = Vec::new();
        for pos in plate_positions {
            if world.block(pos).is_some_and(|b| {
                pressure_plate_kind(&b).is_some_and(|kind| kind.state_signal(&b) == 0)
            }) {
                changed.extend(self.check_pressure_plate(world, pos, now));
            }
        }
        changed
    }

    fn check_pressure_plate(&mut self, world: &mut impl World, pos: Pos, now: u64) -> Vec<Pos> {
        let Some(mut block) = world.block(pos) else {
            return Vec::new();
        };
        let Some(kind) = pressure_plate_kind(&block) else {
            return Vec::new();
        };
        let old = kind.state_signal(&block);
        let contacts = self.plate_contacts.get(&pos).copied().unwrap_or_default();
        let signal = kind.contact_signal(contacts);
        let mut changed = Vec::new();
        if old != signal {
            match kind {
                PlateKind::LightWeighted | PlateKind::HeavyWeighted => {
                    block.properties.insert("power".into(), signal.to_string());
                }
                _ => {
                    block
                        .properties
                        .insert("powered".into(), (signal > 0).to_string());
                }
            }
            world.set_block(pos, Some(block));
            if (old > 0) != (signal > 0) {
                self.sounds.push((
                    pos,
                    UseSound::PressurePlate {
                        material: kind.material(),
                        powered: signal > 0,
                    },
                ));
            }
            changed.push(pos);
            changed.extend(self.changed(world, &[pos, offset(pos, (0, -1, 0))], now));
            changed.sort_unstable();
            changed.dedup();
        }
        if signal > 0 {
            self.ticks
                .schedule(pos, TickKind::PressurePlate, now + kind.pressed_time());
        }
        changed
    }
    /// A cleaned weathering rod is a different block instance. Its onPlace
    /// schedules a second block tick while the old instance's tick remains.
    pub fn copper_replaced_lightning_rod(&mut self, world: &impl World, pos: Pos, now: u64) {
        if world.block(pos).is_some_and(|block| {
            block.id == ROD_IDS[0] && block.property("powered") == Some("true")
        }) {
            self.ticks.schedule(pos, TickKind::LightningRod(0), now + 8);
        }
    }

    /// LightningBolt.powerLightningRod calls LightningRodBlock.onLightningStrike
    /// at the block containing the strike position (bolt Y minus 1e-6).
    /// The block powers immediately and schedules its eight-tick reset.
    pub fn lightning_strike(&mut self, world: &mut impl World, pos: Pos, now: u64) -> Vec<Pos> {
        let Some(mut block) = world.block(pos) else {
            return Vec::new();
        };
        let Some(kind) = rod_tick_kind(&block.id) else {
            return Vec::new();
        };
        block.properties.insert("powered".into(), "true".into());
        world.set_block(pos, Some(block));
        self.ticks.schedule(pos, kind, now + 8);
        let mut positions = vec![pos];
        positions.extend(self.changed(world, &[pos], now));
        positions.sort_unstable();
        positions.dedup();
        positions
    }

    /// NoteBlock.attack calls playNote without cycling NOTE or changing the
    /// block state. The caller still continues its ordinary mining path.
    pub fn attack_block(&mut self, world: &impl World, pos: Pos) -> bool {
        let Some(block) = world.block(pos) else {
            return false;
        };
        if block.id != "minecraft:note_block" {
            return false;
        }
        if note_can_play(world, pos, &block) {
            self.note_events.push(note_event(pos, &block));
        }
        true
    }

    pub fn comparator_output(&self, pos: Pos) -> u8 {
        self.comparator_output.get(&pos).copied().unwrap_or(0)
    }

    pub fn container_signal_at(&self, pos: Pos) -> Option<u8> {
        self.container_signal.get(&pos).copied()
    }

    /// Called from the detector block-entity ticker when game time is a
    /// multiple of 20. The caller supplies the sampled environment values.
    pub fn update_daylight_detector(
        &mut self,
        world: &mut impl World,
        pos: Pos,
        effective_sky: u8,
        sun_angle_degrees: f32,
        now: u64,
    ) -> Vec<Pos> {
        let Some(mut block) = world.block(pos) else {
            return Vec::new();
        };
        if block.id != "minecraft:daylight_detector" {
            return Vec::new();
        }
        let power = detector_power(
            effective_sky,
            sun_angle_degrees,
            block.property("inverted") == Some("true"),
        );
        if block
            .property("power")
            .and_then(|value| value.parse::<u8>().ok())
            == Some(power)
        {
            return Vec::new();
        }
        block.properties.insert("power".into(), power.to_string());
        world.set_block(pos, Some(block));
        let mut positions = vec![pos];
        positions.extend(self.changed(world, &[pos], now));
        positions.sort_unstable();
        positions.dedup();
        positions
    }

    /// `DaylightDetectorBlock.useWithoutItem` cycles INVERTED then immediately
    /// recomputes POWER from the same environment sample.
    pub fn use_daylight_detector(
        &mut self,
        world: &mut impl World,
        pos: Pos,
        effective_sky: u8,
        sun_angle_degrees: f32,
        now: u64,
    ) -> Option<Vec<Pos>> {
        let mut block = world.block(pos)?;
        if block.id != "minecraft:daylight_detector" {
            return None;
        }
        let inverted = block.property("inverted") != Some("true");
        block
            .properties
            .insert("inverted".into(), inverted.to_string());
        world.set_block(pos, Some(block));
        let mut positions = vec![pos];
        positions.extend(self.update_daylight_detector(
            world,
            pos,
            effective_sky,
            sun_angle_degrees,
            now,
        ));
        positions.sort_unstable();
        positions.dedup();
        Some(positions)
    }

    /// Called after a container's own contents-change notification. An
    /// inventory-edit command alone does not notify neighboring comparators.
    pub fn container_changed(
        &mut self,
        world: &mut impl World,
        pos: Pos,
        signal: u8,
        now: u64,
    ) -> Vec<Pos> {
        let signal = signal.min(15);
        if self.container_signal.insert(pos, signal) == Some(signal) {
            return Vec::new();
        }
        self.changed(world, &[pos], now)
    }

    pub fn take_sounds(&mut self) -> Vec<(Pos, UseSound)> {
        std::mem::take(&mut self.sounds)
    }

    pub fn take_executed_ticks(&mut self) -> Vec<(Pos, String)> {
        std::mem::take(&mut self.executed_ticks)
    }

    pub fn take_note_events(&mut self) -> Vec<NoteEvent> {
        std::mem::take(&mut self.note_events)
    }

    pub fn take_piston_events(&mut self) -> Vec<PistonEvent> {
        std::mem::take(&mut self.piston_events)
    }

    pub fn take_dispense_events(&mut self) -> Vec<DispenseEvent> {
        std::mem::take(&mut self.dispense_events)
    }

    pub fn piston_visuals(&self) -> Vec<PistonVisual> {
        let mut visuals: Vec<_> = self.piston_visuals.values().cloned().collect();
        visuals.sort_by_key(|visual| visual.pos);
        visuals
    }

    pub fn has_due(&self, now: u64) -> bool {
        self.ticks.has_due(now)
    }

    /// `now` is the current world tick before the next scheduled-tick phase.
    /// Returns changed block positions for mesh/light invalidation.
    pub fn tick(&mut self, world: &mut impl World, now: u64) -> Vec<Pos> {
        let mut changed = Vec::new();
        self.executed_ticks.clear();
        while let Some((pos, kind)) = self.ticks.pop_due(now) {
            if kind == TickKind::PistonPayloadFinish {
                self.piston_visuals.remove(&pos);
                if let Some(pushed) = self.piston_payloads.remove(&pos) {
                    if world
                        .block(pos)
                        .is_some_and(|b| b.id == "minecraft:moving_piston")
                    {
                        world.set_block(pos, Some(pushed));
                        changed.push(pos);
                    }
                }
                continue;
            }
            if kind == TickKind::PistonExtendFinish {
                let head_visual = self
                    .piston_motion
                    .get(&pos)
                    .filter(|m| m.extending)
                    .map(|m| offset(pos, piston_delta(&m.facing)));
                if let Some(head) = head_visual {
                    self.piston_visuals.remove(&head);
                }
                if self.piston_motion.get(&pos).is_some_and(|m| m.extending) {
                    let motion = self.piston_motion.remove(&pos).unwrap();
                    let head = offset(pos, piston_delta(&motion.facing));
                    if world
                        .block(head)
                        .is_some_and(|b| b.id == "minecraft:moving_piston")
                    {
                        world.set_block(
                            head,
                            Some(
                                Block::new("minecraft:piston_head")
                                    .with("facing", &motion.facing)
                                    .with("short", "false")
                                    .with("type", if motion.sticky { "sticky" } else { "normal" }),
                            ),
                        );
                        changed.push(head);
                    }
                }
                continue;
            }
            if kind == TickKind::PistonRetractFinish {
                self.piston_visuals.remove(&pos);
                if self.piston_motion.get(&pos).is_some_and(|m| !m.extending) {
                    let motion = self.piston_motion.remove(&pos).unwrap();
                    if world
                        .block(pos)
                        .is_some_and(|b| b.id == "minecraft:moving_piston")
                    {
                        world.set_block(
                            pos,
                            Some(
                                Block::new(if motion.sticky {
                                    "minecraft:sticky_piston"
                                } else {
                                    "minecraft:piston"
                                })
                                .with("extended", "false")
                                .with("facing", &motion.facing),
                            ),
                        );
                        changed.push(pos);
                    }
                }
                continue;
            }
            let Some(mut block) = world.block(pos) else {
                continue;
            };
            let event_id = match kind {
                TickKind::LightningRod(which) => ROD_IDS[which as usize].to_owned(),
                TickKind::Dispenser(true) => "minecraft:dropper".to_owned(),
                TickKind::Dispenser(false) => "minecraft:dispenser".to_owned(),
                _ => block.id.clone(),
            };
            self.executed_ticks.push((pos, event_id));
            match kind {
                TickKind::LightningRod(which) if block.id == ROD_IDS[which as usize] => {
                    block.properties.insert("powered".into(), "false".into());
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                }
                TickKind::Lamp if block.id == "minecraft:redstone_lamp" => {
                    if block.property("lit") == Some("true")
                        && !has_neighbor_signal(world, pos, &self.comparator_output)
                    {
                        block.properties.insert("lit".into(), "false".into());
                        world.set_block(pos, Some(block));
                        changed.push(pos);
                    }
                }
                TickKind::PressurePlate if pressure_plate_kind(&block).is_some() => {
                    if pressure_plate_kind(&block).is_some_and(|kind| kind.state_signal(&block) > 0)
                    {
                        changed.extend(self.check_pressure_plate(world, pos, now));
                    }
                }
                TickKind::Button if block.id.ends_with("_button") => {
                    if block.property("powered") == Some("true") {
                        let sound = if matches!(
                            block.id.as_str(),
                            "minecraft:stone_button" | "minecraft:polished_blackstone_button"
                        ) {
                            UseSound::StoneButton { powered: false }
                        } else {
                            UseSound::WoodButton { powered: false }
                        };
                        block.properties.insert("powered".into(), "false".into());
                        world.set_block(pos, Some(block));
                        self.sounds.push((pos, sound));
                        changed.push(pos);
                    }
                }
                TickKind::Torch | TickKind::TorchRestart
                    if matches!(
                        block.id.as_str(),
                        "minecraft:redstone_torch" | "minecraft:redstone_wall_torch"
                    ) =>
                {
                    let lit = block.property("lit") == Some("true");
                    let powered = torch_input(world, pos, &block);
                    self.torch_toggles
                        .retain(|&(_, at)| now.saturating_sub(at) <= 60);
                    if lit && powered {
                        block.properties.insert("lit".into(), "false".into());
                        world.set_block(pos, Some(block));
                        changed.push(pos);
                        self.torch_toggles.push_back((pos, now));
                        if self
                            .torch_toggles
                            .iter()
                            .filter(|&&(at_pos, _)| at_pos == pos)
                            .count()
                            >= 8
                        {
                            // The long burnout check coexists with a nearer
                            // neighbor-triggered torch check at the same pos.
                            self.ticks.schedule(pos, TickKind::TorchRestart, now + 160);
                        }
                    } else if !lit
                        && !powered
                        && self
                            .torch_toggles
                            .iter()
                            .filter(|&&(at_pos, _)| at_pos == pos)
                            .count()
                            < 8
                    {
                        block.properties.insert("lit".into(), "true".into());
                        world.set_block(pos, Some(block));
                        changed.push(pos);
                    }
                }
                TickKind::Repeater if block.id == "minecraft:repeater" => {
                    if repeater_locked(world, pos, &block) {
                        continue;
                    }
                    let powered = block.property("powered") == Some("true");
                    let input = repeater_input(world, pos, &block, &self.comparator_output);
                    if powered && !input {
                        block.properties.insert("powered".into(), "false".into());
                        world.set_block(pos, Some(block));
                        changed.push(pos);
                    } else if !powered {
                        block.properties.insert("powered".into(), "true".into());
                        world.set_block(pos, Some(block.clone()));
                        changed.push(pos);
                        if !input {
                            self.ticks.schedule_with_priority(
                                pos,
                                TickKind::Repeater,
                                now + repeater_delay(&block),
                                -2,
                            );
                        }
                    }
                }
                TickKind::Comparator if block.id == "minecraft:comparator" => {
                    let output = comparator_target_output(
                        world,
                        pos,
                        &block,
                        &self.comparator_output,
                        &self.container_signal,
                    );
                    let powered = comparator_should_turn_on(
                        world,
                        pos,
                        &block,
                        &self.comparator_output,
                        &self.container_signal,
                    );
                    let previous = self.comparator_output.get(&pos).copied().unwrap_or(0);
                    if output == 0 {
                        self.comparator_output.remove(&pos);
                    } else {
                        self.comparator_output.insert(pos, output);
                    }
                    let state_changed =
                        block.property("powered") != Some(if powered { "true" } else { "false" });
                    if state_changed {
                        block
                            .properties
                            .insert("powered".into(), powered.to_string());
                        world.set_block(pos, Some(block));
                    }
                    if state_changed || output != previous {
                        changed.push(pos);
                    }
                }
                TickKind::Observer if block.id == "minecraft:observer" => {
                    let was_powered = block.property("powered") == Some("true");
                    block
                        .properties
                        .insert("powered".into(), (!was_powered).to_string());
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                    if !was_powered {
                        self.ticks.schedule(pos, TickKind::Observer, now + 2);
                    }
                }
                TickKind::Dispenser(dropper)
                    if block.id
                        == if dropper {
                            "minecraft:dropper"
                        } else {
                            "minecraft:dispenser"
                        } =>
                {
                    self.dispense_events.push(DispenseEvent {
                        pos,
                        block_id: block.id.clone(),
                        facing: block.property("facing").unwrap_or("north").to_owned(),
                    });
                }
                _ => {}
            }
        }
        if !changed.is_empty() {
            let more = self.changed(world, &changed, now);
            changed.extend(more);
            changed.sort_unstable();
            changed.dedup();
        }
        changed
    }

    pub fn use_block(
        &mut self,
        world: &mut impl World,
        pos: Pos,
        now: u64,
    ) -> Option<(UseSound, Vec<Pos>)> {
        self.use_block_facing(world, pos, now, "south")
    }

    pub fn use_block_facing(
        &mut self,
        world: &mut impl World,
        pos: Pos,
        now: u64,
        player_facing: &str,
    ) -> Option<(UseSound, Vec<Pos>)> {
        let mut block = world.block(pos)?;
        let powered = block.property("powered") == Some("true");
        let sound = match block.id.as_str() {
            "minecraft:lever" => UseSound::Lever { powered: !powered },
            id if id.ends_with("_button") && !powered => {
                let wooden = !matches!(
                    id,
                    "minecraft:stone_button" | "minecraft:polished_blackstone_button"
                );
                self.ticks
                    .schedule(pos, TickKind::Button, now + if wooden { 30 } else { 20 });
                if wooden {
                    UseSound::WoodButton { powered: true }
                } else {
                    UseSound::StoneButton { powered: true }
                }
            }
            "minecraft:repeater" => {
                let delay = block
                    .property("delay")
                    .and_then(|value| value.parse::<u8>().ok())
                    .unwrap_or(1)
                    .clamp(1, 4);
                block
                    .properties
                    .insert("delay".into(), (delay % 4 + 1).to_string());
                world.set_block(pos, Some(block));
                let mut changed = vec![pos];
                changed.extend(self.changed(world, &[pos], now));
                changed.sort_unstable();
                changed.dedup();
                return Some((UseSound::None, changed));
            }
            "minecraft:comparator" => {
                // ComparatorBlock.useWithoutItem writes MODE, then calls
                // refreshOutputState immediately rather than waiting for a
                // scheduled comparator check.
                let subtract = block.property("mode") != Some("subtract");
                block.properties.insert(
                    "mode".into(),
                    if subtract { "subtract" } else { "compare" }.into(),
                );
                let output = comparator_target_output(
                    world,
                    pos,
                    &block,
                    &self.comparator_output,
                    &self.container_signal,
                );
                let powered = comparator_should_turn_on(
                    world,
                    pos,
                    &block,
                    &self.comparator_output,
                    &self.container_signal,
                );
                block
                    .properties
                    .insert("powered".into(), powered.to_string());
                world.set_block(pos, Some(block));
                if output == 0 {
                    self.comparator_output.remove(&pos);
                } else {
                    self.comparator_output.insert(pos, output);
                }
                let mut changed = vec![pos];
                changed.extend(self.changed(world, &[pos], now));
                changed.sort_unstable();
                changed.dedup();
                return Some((UseSound::Comparator { subtract }, changed));
            }
            "minecraft:note_block" => {
                // NoteBlock.useWithoutItem cycles NOTE and requests a block
                // event even while the block is redstone-powered.
                let note = block
                    .property("note")
                    .and_then(|value| value.parse::<u8>().ok())
                    .unwrap_or(0)
                    .min(24);
                block
                    .properties
                    .insert("note".into(), ((note + 1) % 25).to_string());
                world.set_block(pos, Some(block.clone()));
                let mut changed = vec![pos];
                changed.extend(self.changed(world, &[pos], now));
                if note_can_play(world, pos, &block) {
                    self.note_events.push(note_event(pos, &block));
                }
                changed.sort_unstable();
                changed.dedup();
                return Some((UseSound::None, changed));
            }
            "minecraft:oak_trapdoor" => {
                let open = block.property("open") != Some("true");
                block.properties.insert("open".into(), open.to_string());
                world.set_block(pos, Some(block));
                let mut changed = vec![pos];
                changed.extend(self.changed(world, &[pos], now));
                changed.sort_unstable();
                changed.dedup();
                return Some((UseSound::OakTrapdoor { open }, changed));
            }
            "minecraft:oak_fence_gate" => {
                // FenceGateBlock.useWithoutItem closes in place. Opening
                // from behind flips FACING to the player's direction.
                let open = block.property("open") != Some("true");
                if open && block.property("facing") == Some(opposite(player_facing)) {
                    block
                        .properties
                        .insert("facing".into(), player_facing.into());
                }
                block.properties.insert("open".into(), open.to_string());
                world.set_block(pos, Some(block));
                let mut changed = vec![pos];
                changed.extend(self.changed(world, &[pos], now));
                changed.sort_unstable();
                changed.dedup();
                return Some((UseSound::OakFenceGate { open }, changed));
            }
            "minecraft:oak_door" => {
                // DoorBlock.useWithoutItem cycles OPEN on the touched half;
                // updateShape copies OPEN to its counterpart, leaving POWERED.
                let open = block.property("open") != Some("true");
                let other = if block.property("half") == Some("upper") {
                    offset(pos, (0, -1, 0))
                } else {
                    offset(pos, (0, 1, 0))
                };
                block.properties.insert("open".into(), open.to_string());
                world.set_block(pos, Some(block));
                let mut positions = vec![pos];
                if let Some(mut partner) =
                    world.block(other).filter(|b| b.id == "minecraft:oak_door")
                {
                    partner.properties.insert("open".into(), open.to_string());
                    world.set_block(other, Some(partner));
                    positions.push(other);
                }
                let affected = self.changed(world, &positions, now);
                positions.extend(affected);
                positions.sort_unstable();
                positions.dedup();
                return Some((UseSound::OakDoor { open }, positions));
            }
            _ => return None,
        };
        let next = !powered;
        block.properties.insert("powered".into(), next.to_string());
        world.set_block(pos, Some(block));
        let mut changed = vec![pos];
        changed.extend(self.changed(world, &[pos], now));
        changed.sort_unstable();
        changed.dedup();
        Some((sound, changed))
    }

    /// Re-evaluate affected devices after a world edit. Wire propagation is
    /// still limited to the measured layouts; observer watched faces cover
    /// all six directions.
    pub fn changed(&mut self, world: &mut impl World, positions: &[Pos], now: u64) -> Vec<Pos> {
        for &pos in positions {
            if !world.block(pos).is_some_and(|block| {
                matches!(
                    block.id.as_str(),
                    "minecraft:hopper"
                        | "minecraft:chest"
                        | "minecraft:dropper"
                        | "minecraft:dispenser"
                )
            }) {
                self.container_signal.remove(&pos);
            }
        }
        // RailBlock.updateState reacts to a signal-source neighbor only at
        // three-way junctions. Signal-source membership is from a repeated
        // pinned default-state catalog; air removal is not a signal source.
        let mut rail_shape_changes = Vec::new();
        for &source in positions {
            if !world
                .block(source)
                .is_some_and(|block| is_signal_source(&block))
            {
                continue;
            }
            for direction in [
                (0, 0, -1),
                (1, 0, 0),
                (0, 0, 1),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
            ] {
                let pos = offset(source, direction);
                if world
                    .block(pos)
                    .is_some_and(|block| block.id == "minecraft:rail")
                {
                    let has_signal = has_neighbor_signal(world, pos, &self.comparator_output);
                    rail_shape_changes
                        .extend(crate::rail::signal_neighbor_changed(world, pos, has_signal));
                }
            }
        }
        // ObserverBlock.updateShape only starts a pulse for a change on the
        // watched face. Its two-tick scheduled pulse is suppressed while on.
        for &changed in positions {
            for (dx, dy, dz, facing) in OBSERVER_DIRECTIONS {
                let observer_pos = offset(changed, (-dx, -dy, -dz));
                if world.block(observer_pos).is_some_and(|block| {
                    block.id == "minecraft:observer"
                        && block.property("facing") == Some(facing)
                        && block.property("powered") != Some("true")
                }) {
                    self.ticks
                        .schedule(observer_pos, TickKind::Observer, now + 2);
                }
            }
        }
        let mut queue = VecDeque::new();
        let mut queued = HashSet::new();
        for &pos in positions.iter().chain(rail_shape_changes.iter()) {
            enqueue_around(pos, &mut queue, &mut queued);
            // Level.updateNeighbourForOutputSignal routes an analog source's
            // change through one conducting block to the comparator behind it.
            // Removing the source does not take this path; vanilla can retain
            // the previous output until another neighbor update arrives.
            let analog_source = world
                .block(pos)
                .is_some_and(|b| analog_output(&b).is_some())
                || self.container_signal.contains_key(&pos);
            if analog_source {
                for &(dx, _, dz, direction) in &HORIZONTAL {
                    let between = offset(pos, (dx, 0, dz));
                    if !world
                        .block(between)
                        .is_some_and(|b| is_tested_conductor(&b))
                    {
                        continue;
                    }
                    let comparator = offset(between, (dx, 0, dz));
                    if world.block(comparator).is_some_and(|b| {
                        b.id == "minecraft:comparator"
                            && b.property("facing") == Some(opposite(direction))
                    }) && queued.insert(comparator)
                    {
                        queue.push_back(comparator);
                    }
                }
            }
        }
        let mut changed = rail_shape_changes;
        let mut conductor_seen = HashSet::new();
        let mut door_sound_emitted = HashSet::new();
        let mut steps = 0;
        while let Some(pos) = queue.pop_front() {
            queued.remove(&pos);
            steps += 1;
            if steps > 4096 {
                break;
            }
            let Some(mut block) = world.block(pos) else {
                if self.conductor_power.remove(&pos).is_some() {
                    enqueue_around(pos, &mut queue, &mut queued);
                }
                if self.comparator_output.remove(&pos).is_some() {
                    enqueue_around(pos, &mut queue, &mut queued);
                }
                continue;
            };
            if pressure_plate_kind(&block).is_some()
                && positions.contains(&offset(pos, (0, -1, 0)))
                && world
                    .block(offset(pos, (0, -1, 0)))
                    .is_none_or(|support| support.id == "minecraft:air")
            {
                // BasePressurePlateBlock.updateShape returns air when its
                // support below is removed; powered removal notifies the
                // neighboring circuit through affectNeighborsAfterRemoval.
                world.set_block(pos, None);
                self.plate_contacts.remove(&pos);
                changed.push(pos);
                enqueue_around(pos, &mut queue, &mut queued);
                continue;
            }
            if crate::rail::is_rail(&block) {
                // BaseRailBlock.neighborChanged removes a rail as soon as
                // rigid support is lost below it, or at the uphill side of
                // an ascending shape. The removal is not a scheduled tick.
                let uphill = match block.property("shape") {
                    Some("ascending_east") => Some((1, 0, 0)),
                    Some("ascending_west") => Some((-1, 0, 0)),
                    Some("ascending_north") => Some((0, 0, -1)),
                    Some("ascending_south") => Some((0, 0, 1)),
                    _ => None,
                };
                let supported_below = world
                    .block(offset(pos, (0, -1, 0)))
                    .is_some_and(|support| crate::supports_rigid_top(&support));
                let supported_uphill = uphill.is_none_or(|delta| {
                    world
                        .block(offset(pos, delta))
                        .is_some_and(|support| crate::supports_rigid_top(&support))
                });
                if !supported_below || !supported_uphill {
                    world.set_block(pos, None);
                    changed.push(pos);
                    enqueue_around(pos, &mut queue, &mut queued);
                    continue;
                }
            }
            if matches!(
                block.id.as_str(),
                "minecraft:redstone_torch" | "minecraft:redstone_wall_torch"
            ) {
                let support_delta = if block.id == "minecraft:redstone_wall_torch" {
                    horizontal_delta(opposite(block.property("facing").unwrap_or("north")))
                } else {
                    (0, -1, 0)
                };
                if world
                    .block(offset(pos, support_delta))
                    .is_none_or(|support| support.id == "minecraft:air")
                {
                    world.set_block(pos, None);
                    changed.push(pos);
                    enqueue_around(pos, &mut queue, &mut queued);
                    continue;
                }
            }
            if block.id != "minecraft:comparator" && self.comparator_output.remove(&pos).is_some() {
                enqueue_around(pos, &mut queue, &mut queued);
            }
            if !is_tested_conductor(&block) && self.conductor_power.remove(&pos).is_some() {
                enqueue_around(pos, &mut queue, &mut queued);
            }
            if block.id == "minecraft:redstone_wire" {
                let strength = target_wire_power(world, pos, &self.comparator_output);
                let mut state_changed = block.property("power") != Some(&strength.to_string());
                block
                    .properties
                    .insert("power".into(), strength.to_string());
                let mut connections = [false; 4];
                let mut connection_sides = ["none"; 4];
                for (index, &(dx, dy, dz, direction)) in HORIZONTAL.iter().enumerate() {
                    connection_sides[index] =
                        wire_connection_side(world, pos, (dx, dy, dz), direction);
                    connections[index] = connection_sides[index] != "none";
                }
                // getConnectionState extends a lone straight arm to its
                // opposite side, without adding arms to corners or branches.
                if connections.iter().any(|&connected| connected) {
                    if !connections[0] && !connections[2] {
                        connections[1] = true;
                        connections[3] = true;
                    }
                    if !connections[1] && !connections[3] {
                        connections[0] = true;
                        connections[2] = true;
                    }
                }
                for (index, &(_, _, _, direction)) in HORIZONTAL.iter().enumerate() {
                    let connection = if connection_sides[index] == "none" && connections[index] {
                        "side"
                    } else {
                        connection_sides[index]
                    };
                    if block.property(direction) != Some(connection) {
                        block.properties.insert(direction.into(), connection.into());
                        state_changed = true;
                    }
                }
                if state_changed {
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                    enqueue_around(pos, &mut queue, &mut queued);
                    enqueue_wire_diagonals(pos, &mut queue, &mut queued);
                }
            } else if matches!(
                block.id.as_str(),
                "minecraft:powered_rail" | "minecraft:activator_rail"
            ) {
                // PoweredRailBlock.updateState changes immediately on a
                // neighbor notification. A powered rail can conduct the
                // signal through at most eight connected rails beyond its
                // directly powered source.
                let powered = has_neighbor_signal(world, pos, &self.comparator_output)
                    || rail_chain_power(world, pos, &block, true, 0, &self.comparator_output)
                    || rail_chain_power(world, pos, &block, false, 0, &self.comparator_output);
                if block.property("powered") != Some(if powered { "true" } else { "false" }) {
                    let sloped = block
                        .property("shape")
                        .is_some_and(|shape| shape.starts_with("ascending_"));
                    block
                        .properties
                        .insert("powered".into(), powered.to_string());
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                    enqueue_around(pos, &mut queue, &mut queued);
                    // updateState notifies all neighbors of the support
                    // below, and additionally those above a sloped rail.
                    enqueue_around(offset(pos, (0, -1, 0)), &mut queue, &mut queued);
                    if sloped {
                        enqueue_around(offset(pos, (0, 1, 0)), &mut queue, &mut queued);
                    }
                }
            } else if block.id == "minecraft:hopper" {
                // HopperBlock.checkPoweredState copies the inverse of the
                // current neighbor signal to ENABLED on each notification.
                let enabled = !has_neighbor_signal(world, pos, &self.comparator_output);
                if block.property("enabled") != Some(if enabled { "true" } else { "false" }) {
                    block
                        .properties
                        .insert("enabled".into(), enabled.to_string());
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                }
            } else if matches!(
                block.id.as_str(),
                "minecraft:dispenser" | "minecraft:dropper"
            ) {
                // DispenserBlock.neighborChanged checks both the block and
                // the block above for quasi-connectivity. It does not cancel
                // the four-tick activation when power falls before the tick.
                let powered = has_neighbor_signal(world, pos, &self.comparator_output)
                    || has_neighbor_signal(world, offset(pos, (0, 1, 0)), &self.comparator_output);
                let triggered = block.property("triggered") == Some("true");
                if powered != triggered {
                    if powered {
                        self.ticks.schedule(
                            pos,
                            TickKind::Dispenser(block.id == "minecraft:dropper"),
                            now + 4,
                        );
                    }
                    block
                        .properties
                        .insert("triggered".into(), powered.to_string());
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                }
            } else if block.id.ends_with("copper_bulb") {
                // CopperBulbBlock.checkAndFlip: only a change in POWERED
                // updates the state; the rising edge cycles LIT immediately.
                let was_powered = block.property("powered") == Some("true");
                let powered = has_neighbor_signal(world, pos, &self.comparator_output);
                if powered != was_powered {
                    if !was_powered {
                        let lit = block.property("lit") != Some("true");
                        block.properties.insert("lit".into(), lit.to_string());
                        self.sounds.push((pos, UseSound::CopperBulb { lit }));
                    }
                    block
                        .properties
                        .insert("powered".into(), powered.to_string());
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                    enqueue_around(pos, &mut queue, &mut queued);
                }
            } else if block.id == "minecraft:redstone_lamp" {
                let lit = block.property("lit") == Some("true");
                let powered = has_neighbor_signal(world, pos, &self.comparator_output);
                if powered && !lit {
                    block.properties.insert("lit".into(), "true".into());
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                } else if lit && !powered {
                    // RedstoneLampBlock.neighborChanged schedules a four-tick
                    // off check; renewed power is checked when that tick fires.
                    self.ticks.schedule(pos, TickKind::Lamp, now + 4);
                }
            } else if matches!(
                block.id.as_str(),
                "minecraft:redstone_torch" | "minecraft:redstone_wall_torch"
            ) {
                let lit = block.property("lit") == Some("true");
                if lit == torch_input(world, pos, &block) {
                    // RedstoneTorchBlock.neighborChanged schedules a two-tick
                    // check when its output disagrees with input from below.
                    self.ticks.schedule(pos, TickKind::Torch, now + 2);
                }
            } else if block.id == "minecraft:note_block" {
                // NoteBlock.updateShape refreshes INSTRUMENT for a vertical
                // neighbor change, not on horizontal power or hand tuning.
                if positions.contains(&offset(pos, (0, 1, 0)))
                    || positions.contains(&offset(pos, (0, -1, 0)))
                {
                    let instrument = note_top_instrument(world, pos).or_else(|| {
                        match world
                            .block(offset(pos, (0, -1, 0)))
                            .as_ref()
                            .map(|b| b.id.as_str())
                        {
                            Some("minecraft:stone") => Some("basedrum"),
                            None | Some("minecraft:air") => Some("harp"),
                            _ => None,
                        }
                    });
                    if let Some(instrument) = instrument {
                        if block.property("instrument") != Some(instrument) {
                            block
                                .properties
                                .insert("instrument".into(), instrument.into());
                            world.set_block(pos, Some(block.clone()));
                            changed.push(pos);
                        }
                    }
                }
                let powered = has_neighbor_signal(world, pos, &self.comparator_output);
                if block.property("powered") != Some(if powered { "true" } else { "false" }) {
                    if powered && note_can_play(world, pos, &block) {
                        self.note_events.push(note_event(pos, &block));
                    }
                    block
                        .properties
                        .insert("powered".into(), powered.to_string());
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                    enqueue_around(pos, &mut queue, &mut queued);
                }
            } else if block.id == "minecraft:piston" || block.id == "minecraft:sticky_piston" {
                let sticky = block.id == "minecraft:sticky_piston";
                let facing = block.property("facing").unwrap_or("north").to_owned();
                let powered = piston_has_signal(world, pos, &facing, &self.comparator_output);
                let extended = block.property("extended") == Some("true");
                if powered && !extended {
                    let delta = piston_delta(&facing);
                    let head = offset(pos, delta);
                    // Source PistonStructureResolver caps ordinary forward
                    // movement at 12 blocks. This measured slice only accepts
                    // stone and air; other push reactions remain separate.
                    let mut chain = Vec::new();
                    let mut cursor = head;
                    let mut can_extend = false;
                    for _ in 0..=12 {
                        match world.block(cursor) {
                            None => {
                                can_extend = true;
                                break;
                            }
                            Some(pushed) if pushed.id == "minecraft:stone" && chain.len() < 12 => {
                                chain.push((cursor, pushed));
                                cursor = offset(cursor, delta);
                            }
                            _ => break,
                        }
                    }
                    if !can_extend {
                        continue;
                    }
                    for _ in 0..2 {
                        self.piston_events.push(PistonEvent {
                            pos,
                            id: 0,
                            param: piston_param(&facing),
                        });
                    }
                    block.properties.insert("extended".into(), "true".into());
                    world.set_block(pos, Some(block));
                    world.set_block(head, Some(moving_piston(&facing, sticky)));
                    changed.extend([pos, head]);
                    self.piston_visuals.insert(
                        head,
                        PistonVisual {
                            pos: head,
                            facing: facing.clone(),
                            moved_state: Block::new("minecraft:piston_head")
                                .with("facing", &facing)
                                .with("short", "false")
                                .with("type", if sticky { "sticky" } else { "normal" }),
                            extending: true,
                            source: true,
                            began_at: now,
                        },
                    );
                    for (source, pushed) in chain.into_iter().rev() {
                        let target = offset(source, delta);
                        world.set_block(target, Some(moving_piston(&facing, false)));
                        changed.push(target);
                        self.piston_visuals.insert(
                            target,
                            PistonVisual {
                                pos: target,
                                facing: facing.clone(),
                                moved_state: pushed.clone(),
                                extending: true,
                                source: false,
                                began_at: now,
                            },
                        );
                        self.piston_payloads.insert(target, pushed);
                        self.ticks
                            .schedule(target, TickKind::PistonPayloadFinish, now + 3);
                    }
                    self.piston_motion.insert(
                        pos,
                        PistonMotion {
                            facing,
                            extending: true,
                            sticky,
                        },
                    );
                    self.ticks
                        .schedule(pos, TickKind::PistonExtendFinish, now + 3);
                    self.sounds
                        .push((pos, UseSound::Piston { extending: true }));
                    enqueue_around(pos, &mut queue, &mut queued);
                } else if !powered && extended {
                    let delta = piston_delta(&facing);
                    let head = offset(pos, delta);
                    let fast = self
                        .piston_motion
                        .get(&pos)
                        .is_some_and(|motion| motion.extending);
                    for _ in 0..if fast { 3 } else { 2 } {
                        self.piston_events.push(PistonEvent {
                            pos,
                            id: if fast { 2 } else { 1 },
                            param: piston_param(&facing),
                        });
                    }
                    world.set_block(pos, Some(moving_piston(&facing, sticky)));
                    world.set_block(head, None);
                    changed.extend([pos, head]);
                    self.piston_visuals.remove(&head);
                    self.piston_visuals.insert(
                        pos,
                        PistonVisual {
                            pos,
                            facing: facing.clone(),
                            moved_state: Block::new(if sticky {
                                "minecraft:sticky_piston"
                            } else {
                                "minecraft:piston"
                            })
                            .with("extended", "true")
                            .with("facing", &facing),
                            extending: false,
                            source: true,
                            began_at: now,
                        },
                    );
                    if sticky && fast {
                        // PistonBaseBlock's early sticky retract finalizes an
                        // already extending payload before the base finishes.
                        let trailing = offset(head, delta);
                        if let Some(pushed) = self.piston_payloads.remove(&trailing) {
                            if world
                                .block(trailing)
                                .is_some_and(|b| b.id == "minecraft:moving_piston")
                            {
                                world.set_block(trailing, Some(pushed));
                                changed.push(trailing);
                            }
                            self.piston_visuals.remove(&trailing);
                        }
                    }
                    if sticky && !fast {
                        let pull_from = offset(head, delta);
                        if let Some(pulled) =
                            world.block(pull_from).filter(|b| b.id == "minecraft:stone")
                        {
                            world.set_block(pull_from, None);
                            world.set_block(head, Some(moving_piston(&facing, false)));
                            changed.push(pull_from);
                            self.piston_visuals.insert(
                                head,
                                PistonVisual {
                                    pos: head,
                                    facing: facing.clone(),
                                    moved_state: pulled.clone(),
                                    extending: false,
                                    source: false,
                                    began_at: now,
                                },
                            );
                            self.piston_payloads.insert(head, pulled);
                            self.ticks
                                .schedule(head, TickKind::PistonPayloadFinish, now + 3);
                        }
                    }
                    self.piston_motion.insert(
                        pos,
                        PistonMotion {
                            facing,
                            extending: false,
                            sticky,
                        },
                    );
                    self.ticks
                        .schedule(pos, TickKind::PistonRetractFinish, now + 3);
                    self.sounds
                        .push((pos, UseSound::Piston { extending: false }));
                    enqueue_around(pos, &mut queue, &mut queued);
                }
            } else if block.id == "minecraft:repeater" {
                let locked = repeater_locked(world, pos, &block);
                if block.property("locked") != Some(if locked { "true" } else { "false" }) {
                    block.properties.insert("locked".into(), locked.to_string());
                    world.set_block(pos, Some(block.clone()));
                    changed.push(pos);
                    enqueue_around(pos, &mut queue, &mut queued);
                }
                if !locked {
                    let powered = block.property("powered") == Some("true");
                    if powered != repeater_input(world, pos, &block, &self.comparator_output) {
                        let priority = if diode_should_prioritize(world, pos, &block) {
                            -3
                        } else if powered {
                            -2
                        } else {
                            -1
                        };
                        self.ticks.schedule_with_priority(
                            pos,
                            TickKind::Repeater,
                            now + repeater_delay(&block),
                            priority,
                        );
                    }
                }
            } else if block.id == "minecraft:comparator" {
                let output = self.comparator_output.get(&pos).copied().unwrap_or(0);
                let target = comparator_target_output(
                    world,
                    pos,
                    &block,
                    &self.comparator_output,
                    &self.container_signal,
                );
                let powered = comparator_should_turn_on(
                    world,
                    pos,
                    &block,
                    &self.comparator_output,
                    &self.container_signal,
                );
                if output != target
                    || block.property("powered") != Some(if powered { "true" } else { "false" })
                {
                    let priority = if diode_should_prioritize(world, pos, &block) {
                        -1
                    } else {
                        0
                    };
                    self.ticks
                        .schedule_with_priority(pos, TickKind::Comparator, now + 2, priority);
                }
            } else if block.id.ends_with("_trapdoor") {
                // TrapDoorBlock.neighborChanged mirrors neighbor power into
                // POWERED and, when necessary, OPEN in the same state write.
                let powered = has_neighbor_signal(world, pos, &self.comparator_output);
                if block.property("powered") != Some(if powered { "true" } else { "false" }) {
                    block
                        .properties
                        .insert("powered".into(), powered.to_string());
                    if block.property("open") != Some(if powered { "true" } else { "false" }) {
                        block.properties.insert("open".into(), powered.to_string());
                        if block.id == "minecraft:oak_trapdoor" {
                            self.sounds
                                .push((pos, UseSound::OakTrapdoor { open: powered }));
                        }
                    }
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                }
            } else if block.id.ends_with("_fence_gate") {
                // FenceGateBlock.updateShape recomputes IN_WALL when a wall
                // changes on either side perpendicular to the gate facing.
                let wall_sides = if matches!(block.property("facing"), Some("east" | "west")) {
                    [(0, 0, -1), (0, 0, 1)]
                } else {
                    [(-1, 0, 0), (1, 0, 0)]
                };
                let in_wall = wall_sides.into_iter().any(|delta| {
                    world
                        .block(offset(pos, delta))
                        .is_some_and(|neighbor| neighbor.id.ends_with("_wall"))
                });
                let mut state_changed = false;
                if block.property("in_wall") != Some(if in_wall { "true" } else { "false" }) {
                    block
                        .properties
                        .insert("in_wall".into(), in_wall.to_string());
                    state_changed = true;
                }
                // FenceGateBlock.neighborChanged writes POWERED and OPEN in
                // one update when the adjacent signal changes.
                let powered = has_neighbor_signal(world, pos, &self.comparator_output);
                if block.property("powered") != Some(if powered { "true" } else { "false" }) {
                    block
                        .properties
                        .insert("powered".into(), powered.to_string());
                    state_changed = true;
                    if block.property("open") != Some(if powered { "true" } else { "false" }) {
                        block.properties.insert("open".into(), powered.to_string());
                        if block.id == "minecraft:oak_fence_gate" {
                            self.sounds
                                .push((pos, UseSound::OakFenceGate { open: powered }));
                        }
                    }
                }
                if state_changed {
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                }
            } else if block.id.ends_with("_door") && !block.id.ends_with("_trapdoor") {
                // DoorBlock.neighborChanged checks power at either half.
                // updateShape copies OPEN and POWERED to the partner half.
                let other = if block.property("half") == Some("upper") {
                    offset(pos, (0, -1, 0))
                } else {
                    offset(pos, (0, 1, 0))
                };
                let has_partner = world.block(other).is_some_and(|partner| {
                    partner.id.ends_with("_door")
                        && !partner.id.ends_with("_trapdoor")
                        && partner.property("half") != block.property("half")
                });
                let lost_support = block.property("half") != Some("upper")
                    && world
                        .block(offset(pos, (0, -1, 0)))
                        .is_none_or(|support| support.id == "minecraft:air");
                if !has_partner || lost_support {
                    world.set_block(pos, None);
                    changed.push(pos);
                    enqueue_around(pos, &mut queue, &mut queued);
                    continue;
                }
                let powered = has_neighbor_signal(world, pos, &self.comparator_output)
                    || has_neighbor_signal(world, other, &self.comparator_output);
                if block.property("powered") != Some(if powered { "true" } else { "false" }) {
                    block
                        .properties
                        .insert("powered".into(), powered.to_string());
                    if block.property("open") != Some(if powered { "true" } else { "false" }) {
                        block.properties.insert("open".into(), powered.to_string());
                        if block.id == "minecraft:oak_door" || block.id == "minecraft:iron_door" {
                            let lower = if block.property("half") == Some("upper") {
                                other
                            } else {
                                pos
                            };
                            if door_sound_emitted.insert(lower) {
                                self.sounds.push((
                                    pos,
                                    if block.id == "minecraft:iron_door" {
                                        UseSound::IronDoor { open: powered }
                                    } else {
                                        UseSound::OakDoor { open: powered }
                                    },
                                ));
                            }
                        }
                    }
                    world.set_block(pos, Some(block));
                    changed.push(pos);
                    enqueue_around(pos, &mut queue, &mut queued);
                }
            } else if is_tested_conductor(&block) && conductor_seen.insert(pos) {
                // A conductor relays the strongest adjacent direct signal.
                // Requeue its neighbors only when that relayed value changes.
                let next = direct_signal_to(world, pos, &self.comparator_output);
                let previous = self.conductor_power.get(&pos).copied().unwrap_or(0);
                if next != previous {
                    if next == 0 {
                        self.conductor_power.remove(&pos);
                    } else {
                        self.conductor_power.insert(pos, next);
                    }
                    enqueue_around(pos, &mut queue, &mut queued);
                }
            }
        }
        changed
    }
}

fn offset(pos: Pos, delta: Pos) -> Pos {
    (pos.0 + delta.0, pos.1 + delta.1, pos.2 + delta.2)
}

fn piston_delta(facing: &str) -> Pos {
    match facing {
        "down" => (0, -1, 0),
        "up" => (0, 1, 0),
        "north" => (0, 0, -1),
        "south" => (0, 0, 1),
        "west" => (-1, 0, 0),
        _ => (1, 0, 0),
    }
}

fn piston_param(facing: &str) -> u8 {
    match facing {
        "down" => 0,
        "up" => 1,
        "north" => 2,
        "south" => 3,
        "west" => 4,
        _ => 5,
    }
}

fn moving_piston(facing: &str, sticky: bool) -> Block {
    Block::new("minecraft:moving_piston")
        .with("facing", facing)
        .with("type", if sticky { "sticky" } else { "normal" })
}

fn piston_has_signal(
    world: &impl World,
    pos: Pos,
    facing: &str,
    comparator_output: &HashMap<Pos, u8>,
) -> bool {
    for (direction, delta) in [
        ("down", (0, -1, 0)),
        ("up", (0, 1, 0)),
        ("north", (0, 0, -1)),
        ("south", (0, 0, 1)),
        ("west", (-1, 0, 0)),
        ("east", (1, 0, 0)),
    ] {
        if direction != facing
            && signal_at(world, offset(pos, delta), direction, comparator_output) > 0
        {
            return true;
        }
    }
    let above = offset(pos, (0, 1, 0));
    for (direction, delta) in [
        ("up", (0, 1, 0)),
        ("north", (0, 0, -1)),
        ("south", (0, 0, 1)),
        ("west", (-1, 0, 0)),
        ("east", (1, 0, 0)),
    ] {
        if signal_at(world, offset(above, delta), direction, comparator_output) > 0 {
            return true;
        }
    }
    false
}

fn enqueue_around(pos: Pos, queue: &mut VecDeque<Pos>, queued: &mut HashSet<Pos>) {
    for p in std::iter::once(pos)
        .chain(
            HORIZONTAL
                .iter()
                .map(|&(x, y, z, _)| offset(pos, (x, y, z))),
        )
        .chain([offset(pos, (0, 1, 0)), offset(pos, (0, -1, 0))])
    {
        if queued.insert(p) {
            queue.push_back(p);
        }
    }
}

fn enqueue_wire_diagonals(pos: Pos, queue: &mut VecDeque<Pos>, queued: &mut HashSet<Pos>) {
    for &(dx, _, dz, _) in &HORIZONTAL {
        for dy in [-1, 1] {
            let neighbor = offset(pos, (dx, dy, dz));
            if queued.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }
}

fn wire_connection_side(world: &impl World, pos: Pos, delta: Pos, direction: &str) -> &'static str {
    let adjacent_pos = offset(pos, delta);
    let adjacent = world.block(adjacent_pos);
    let above_clear = !world
        .block(offset(pos, (0, 1, 0)))
        .is_some_and(|block| is_tested_conductor(&block));
    if above_clear
        && adjacent.as_ref().is_some_and(is_tested_conductor)
        && world
            .block(offset(adjacent_pos, (0, 1, 0)))
            .is_some_and(|block| block.id == "minecraft:redstone_wire")
    {
        return "up";
    }
    if adjacent
        .as_ref()
        .is_some_and(|block| connects_to_wire(block, direction))
        || !adjacent.as_ref().is_some_and(is_tested_conductor)
            && world
                .block(offset(adjacent_pos, (0, -1, 0)))
                .is_some_and(|block| block.id == "minecraft:redstone_wire")
    {
        return "side";
    }
    "none"
}

fn connects_to_wire(block: &Block, direction: &str) -> bool {
    block.id == "minecraft:redstone_wire"
        || block.id == "minecraft:lever"
        || pressure_plate_kind(block).is_some()
        || block.id == "minecraft:daylight_detector"
        || block.id.ends_with("_button")
        || block.id == "minecraft:redstone_block"
        || is_lightning_rod(&block.id)
        || block.id == "minecraft:observer" && block.property("facing") == Some(direction)
        // ComparatorBlock inherits BlockBehaviour's signal-source rule,
        // which connects from any horizontal side. RepeaterBlock overrides
        // it and only connects along its facing axis.
        || block.id == "minecraft:repeater"
            && block
                .property("facing")
                .is_some_and(|facing| facing == direction || opposite(facing) == direction)
        || block.id == "minecraft:comparator"
}

fn signal_from(block: &Block, wire_direction: Option<&str>) -> u8 {
    match block.id.as_str() {
        "minecraft:lever" => u8::from(block.property("powered") == Some("true")) * 15,
        id if id.ends_with("_pressure_plate") && pressure_plate_kind(block).is_some() => {
            pressure_plate_kind(block).unwrap().state_signal(block)
        }
        "minecraft:redstone_block" => 15,
        id if is_lightning_rod(id) => u8::from(block.property("powered") == Some("true")) * 15,
        "minecraft:daylight_detector" => block
            .property("power")
            .and_then(|power| power.parse().ok())
            .unwrap_or(0),
        "minecraft:redstone_torch"
            if block.property("lit") == Some("true") && wire_direction != Some("down") =>
        {
            15
        }
        "minecraft:redstone_wall_torch"
            if block.property("lit") == Some("true")
                && wire_direction != block.property("facing").map(opposite) =>
        {
            15
        }
        "minecraft:repeater"
            if block.property("powered") == Some("true")
                && wire_direction.is_some_and(|direction| {
                    block
                        .property("facing")
                        .is_some_and(|facing| opposite(facing) == direction)
                }) =>
        {
            15
        }
        "minecraft:observer"
            if block.property("powered") == Some("true")
                && wire_direction.is_some_and(|direction| {
                    block
                        .property("facing")
                        .is_some_and(|facing| opposite(facing) == direction)
                }) =>
        {
            15
        }
        "minecraft:redstone_wire"
            if wire_direction
                .is_some_and(|direction| block.property(direction) != Some("none")) =>
        {
            block
                .property("power")
                .and_then(|p| p.parse().ok())
                .unwrap_or(0)
        }
        id if id.ends_with("_button") => u8::from(block.property("powered") == Some("true")) * 15,
        _ => 0,
    }
}

fn is_tested_conductor(block: &Block) -> bool {
    matches!(
        block.id.as_str(),
        "minecraft:stone" | "minecraft:cobblestone" | "minecraft:oak_planks"
    )
}

fn direct_signal_from(block: &Block, direction: &str) -> u8 {
    match block.id.as_str() {
        id if id.ends_with("_pressure_plate")
            && pressure_plate_kind(block).is_some()
            && direction == "up" =>
        {
            pressure_plate_kind(block).unwrap().state_signal(block)
        }
        "minecraft:lever" if block.property("powered") == Some("true") => {
            let attached = match block.property("face") {
                Some("floor") => "up",
                Some("ceiling") => "down",
                _ => block.property("facing").unwrap_or("north"),
            };
            u8::from(attached == direction) * 15
        }
        id if is_lightning_rod(id) && block.property("powered") == Some("true") => {
            u8::from(block.property("facing") == Some(direction)) * 15
        }
        "minecraft:redstone_torch" | "minecraft:redstone_wall_torch" if direction == "down" => {
            u8::from(block.property("lit") == Some("true")) * 15
        }
        // DiodeBlock.getDirectSignal delegates to getSignal, whose direction
        // is from the receiving block toward this source.
        "minecraft:repeater"
            if block.property("powered") == Some("true")
                && block.property("facing") == Some(direction) =>
        {
            15
        }
        "minecraft:observer" => signal_from(block, Some(direction)),
        _ => 0,
    }
}

fn direct_signal_to(world: &impl World, pos: Pos, comparator_output: &HashMap<Pos, u8>) -> u8 {
    HORIZONTAL
        .iter()
        .map(|&(x, y, z, direction)| (offset(pos, (x, y, z)), direction))
        .chain([
            (offset(pos, (0, 1, 0)), "up"),
            (offset(pos, (0, -1, 0)), "down"),
        ])
        .filter_map(|(source, direction)| {
            world.block(source).map(|block| {
                if block.id == "minecraft:comparator" {
                    comparator_signal(&block, source, direction, comparator_output)
                } else {
                    direct_signal_from(&block, direction)
                }
            })
        })
        .max()
        .unwrap_or(0)
}

fn signal_at(
    world: &impl World,
    pos: Pos,
    direction: &str,
    comparator_output: &HashMap<Pos, u8>,
) -> u8 {
    world.block(pos).map_or(0, |block| {
        if block.id == "minecraft:comparator" {
            comparator_signal(&block, pos, direction, comparator_output)
        } else if is_tested_conductor(&block) {
            signal_from(&block, Some(direction)).max(direct_signal_to(
                world,
                pos,
                comparator_output,
            ))
        } else {
            signal_from(&block, Some(direction))
        }
    })
}

fn has_neighbor_signal(world: &impl World, pos: Pos, comparator_output: &HashMap<Pos, u8>) -> bool {
    HORIZONTAL
        .iter()
        .map(|&(x, y, z, direction)| {
            let from_neighbor = match direction {
                "north" => "south",
                "east" => "west",
                "south" => "north",
                _ => "east",
            };
            (offset(pos, (x, y, z)), Some(from_neighbor))
        })
        .chain([
            (offset(pos, (0, 1, 0)), Some("down")),
            (offset(pos, (0, -1, 0)), Some("up")),
        ])
        .any(|(p, direction)| {
            direction.is_some_and(|direction| signal_at(world, p, direction, comparator_output) > 0)
        })
}

/// BlockState.isSignalSource for the authored-circuit blocks in the repeated
/// pinned 26.3 catalog. The sampled POWERED/LIT variants share this flag.
pub fn is_signal_source(block: &Block) -> bool {
    matches!(
        block.id.as_str(),
        "minecraft:detector_rail"
            | "minecraft:lever"
            | "minecraft:stone_pressure_plate"
            | "minecraft:oak_pressure_plate"
            | "minecraft:redstone_torch"
            | "minecraft:redstone_wall_torch"
            | "minecraft:stone_button"
            | "minecraft:repeater"
            | "minecraft:oak_button"
            | "minecraft:light_weighted_pressure_plate"
            | "minecraft:heavy_weighted_pressure_plate"
            | "minecraft:comparator"
            | "minecraft:daylight_detector"
            | "minecraft:redstone_block"
            | "minecraft:observer"
            | "minecraft:redstone_wire"
    )
}

/// PoweredRailBlock.findPoweredRailSignal/isSameRailWithPower. The search
/// follows the rail axis, tries a lower rail on flat/downhill segments and
/// stops before examining another rail once depth reaches eight.
fn rail_chain_power(
    world: &impl World,
    pos: Pos,
    rail: &Block,
    forward: bool,
    depth: u8,
    comparator_output: &HashMap<Pos, u8>,
) -> bool {
    if depth >= 8 {
        return false;
    }
    let (step, axis, try_below) = match (rail.property("shape"), forward) {
        (Some("north_south"), true) => ((0, 0, 1), "north_south", true),
        (Some("north_south"), false) => ((0, 0, -1), "north_south", true),
        (Some("east_west"), true) => ((-1, 0, 0), "east_west", true),
        (Some("east_west"), false) => ((1, 0, 0), "east_west", true),
        (Some("ascending_east"), true) => ((-1, 0, 0), "east_west", true),
        (Some("ascending_east"), false) => ((1, 1, 0), "east_west", false),
        (Some("ascending_west"), true) => ((-1, 1, 0), "east_west", false),
        (Some("ascending_west"), false) => ((1, 0, 0), "east_west", true),
        (Some("ascending_north"), true) => ((0, 0, 1), "north_south", true),
        (Some("ascending_north"), false) => ((0, 1, -1), "north_south", false),
        (Some("ascending_south"), true) => ((0, 1, 1), "north_south", false),
        (Some("ascending_south"), false) => ((0, 0, -1), "north_south", true),
        _ => return false,
    };
    let candidate = offset(pos, step);
    rail_candidate_power(
        world,
        candidate,
        forward,
        depth,
        axis,
        &rail.id,
        comparator_output,
    ) || (try_below
        && rail_candidate_power(
            world,
            offset(candidate, (0, -1, 0)),
            forward,
            depth,
            axis,
            &rail.id,
            comparator_output,
        ))
}

fn rail_candidate_power(
    world: &impl World,
    pos: Pos,
    forward: bool,
    depth: u8,
    axis: &str,
    rail_id: &str,
    comparator_output: &HashMap<Pos, u8>,
) -> bool {
    let Some(rail) = world.block(pos).filter(|block| block.id == rail_id) else {
        return false;
    };
    let aligned = match axis {
        "east_west" => matches!(
            rail.property("shape"),
            Some("east_west" | "ascending_east" | "ascending_west")
        ),
        _ => matches!(
            rail.property("shape"),
            Some("north_south" | "ascending_north" | "ascending_south")
        ),
    };
    aligned
        && rail.property("powered") == Some("true")
        && (has_neighbor_signal(world, pos, comparator_output)
            || rail_chain_power(world, pos, &rail, forward, depth + 1, comparator_output))
}

fn torch_input(world: &impl World, pos: Pos, torch: &Block) -> bool {
    // Floor torches query the block below with DOWN; wall torches query the
    // support opposite FACING with that same opposite direction.
    let support_direction = if torch.id == "minecraft:redstone_wall_torch" {
        opposite(torch.property("facing").unwrap_or("north"))
    } else {
        "down"
    };
    let delta = if support_direction == "down" {
        (0, -1, 0)
    } else {
        horizontal_delta(support_direction)
    };
    world
        .block(offset(pos, delta))
        .is_some_and(|block| signal_from(&block, Some(support_direction)) > 0)
}

fn opposite(direction: &str) -> &'static str {
    match direction {
        "north" => "south",
        "east" => "west",
        "south" => "north",
        "west" => "east",
        "up" => "down",
        _ => "up",
    }
}

fn diode_should_prioritize(world: &impl World, pos: Pos, block: &Block) -> bool {
    let output_direction = opposite(block.property("facing").unwrap_or("north"));
    let neighbor = world.block(offset(pos, horizontal_delta(output_direction)));
    neighbor.is_some_and(|neighbor| {
        matches!(
            neighbor.id.as_str(),
            "minecraft:repeater" | "minecraft:comparator"
        ) && neighbor.property("facing") != Some(output_direction)
    })
}

fn note_can_play(world: &impl World, pos: Pos, block: &Block) -> bool {
    let above_is_air = world
        .block(offset(pos, (0, 1, 0)))
        .is_none_or(|above| above.id == "minecraft:air");
    let works_above = matches!(
        block.property("instrument"),
        Some(
            "zombie"
                | "skeleton"
                | "creeper"
                | "dragon"
                | "wither_skeleton"
                | "piglin"
                | "custom_head"
        )
    );
    above_is_air || works_above
}

fn note_top_instrument(world: &impl World, pos: Pos) -> Option<&'static str> {
    let above = world.block(offset(pos, (0, 1, 0)))?;
    match above.id.as_str() {
        "minecraft:zombie_head" | "minecraft:zombie_wall_head" => Some("zombie"),
        "minecraft:skeleton_skull" | "minecraft:skeleton_wall_skull" => Some("skeleton"),
        "minecraft:creeper_head" | "minecraft:creeper_wall_head" => Some("creeper"),
        "minecraft:dragon_head" | "minecraft:dragon_wall_head" => Some("dragon"),
        "minecraft:wither_skeleton_skull" | "minecraft:wither_skeleton_wall_skull" => {
            Some("wither_skeleton")
        }
        "minecraft:piglin_head" | "minecraft:piglin_wall_head" => Some("piglin"),
        "minecraft:player_head" | "minecraft:player_wall_head" => Some("custom_head"),
        _ => None,
    }
}

fn note_event(pos: Pos, block: &Block) -> NoteEvent {
    NoteEvent {
        pos,
        instrument: block.property("instrument").unwrap_or("harp").to_owned(),
        note: block
            .property("note")
            .and_then(|value| value.parse::<u8>().ok())
            .unwrap_or(0)
            .min(24),
    }
}

fn horizontal_delta(direction: &str) -> Pos {
    match direction {
        "north" => (0, 0, -1),
        "east" => (1, 0, 0),
        "south" => (0, 0, 1),
        _ => (-1, 0, 0),
    }
}

fn repeater_delay(block: &Block) -> u64 {
    u64::from(
        block
            .property("delay")
            .and_then(|value| value.parse::<u8>().ok())
            .unwrap_or(1)
            .clamp(1, 4),
    ) * 2
}

fn repeater_input(
    world: &impl World,
    pos: Pos,
    repeater: &Block,
    comparator_output: &HashMap<Pos, u8>,
) -> bool {
    let facing = repeater.property("facing").unwrap_or("north");
    let input = world.block(offset(pos, horizontal_delta(facing)));
    input.is_some_and(|block| {
        signal_at(
            world,
            offset(pos, horizontal_delta(facing)),
            facing,
            comparator_output,
        ) > 0
            || block.id == "minecraft:redstone_wire"
                && block
                    .property("power")
                    .and_then(|power| power.parse::<u8>().ok())
                    .unwrap_or(0)
                    > 0
    })
}

fn repeater_locked(world: &impl World, pos: Pos, repeater: &Block) -> bool {
    // RepeaterBlock accepts alternate control input only from adjacent diodes.
    // The two side positions are perpendicular to the input-facing direction.
    let facing = repeater.property("facing").unwrap_or("north");
    let side_positions: [(Pos, &str); 2] = if facing == "north" || facing == "south" {
        [((1, 0, 0), "west"), ((-1, 0, 0), "east")]
    } else {
        [((0, 0, 1), "north"), ((0, 0, -1), "south")]
    };
    side_positions
        .into_iter()
        .any(|(delta, output_toward_repeater)| {
            world.block(offset(pos, delta)).is_some_and(|block| {
                block.id == "minecraft:repeater"
                    && signal_from(&block, Some(output_toward_repeater)) > 0
            })
        })
}

fn comparator_signal(
    block: &Block,
    pos: Pos,
    direction: &str,
    comparator_output: &HashMap<Pos, u8>,
) -> u8 {
    if block.property("powered") == Some("true")
        && block
            .property("facing")
            .is_some_and(|facing| opposite(facing) == direction)
    {
        comparator_output.get(&pos).copied().unwrap_or(0)
    } else {
        0
    }
}

fn comparator_input(
    world: &impl World,
    pos: Pos,
    block: &Block,
    comparator_output: &HashMap<Pos, u8>,
    container_signal: &HashMap<Pos, u8>,
) -> u8 {
    let facing = block.property("facing").unwrap_or("north");
    let input_pos = offset(pos, horizontal_delta(facing));
    let signal = signal_at(world, input_pos, facing, comparator_output);
    if let Some(input) = world.block(input_pos) {
        // ComparatorBlock.getInputSignal replaces the ordinary rear signal
        // with a directly adjacent block's analog output when it has one.
        if let Some(analog) = analog_output(&input) {
            return analog;
        }
        if let Some(&analog) = container_signal.get(&input_pos) {
            return analog;
        }
        if signal < 15 && is_tested_conductor(&input) {
            let beyond = offset(input_pos, horizontal_delta(facing));
            if let Some(source) = world.block(beyond) {
                if let Some(analog) = analog_output(&source) {
                    return analog;
                }
                if let Some(&analog) = container_signal.get(&beyond) {
                    return analog;
                }
            }
        }
        if input.id == "minecraft:redstone_wire" {
            return signal.max(
                input
                    .property("power")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
            );
        }
    }
    signal
}

fn analog_output(block: &Block) -> Option<u8> {
    if block.id.ends_with("copper_bulb") {
        return Some(if block.property("lit") == Some("true") {
            15
        } else {
            0
        });
    }
    match block.id.as_str() {
        "minecraft:cake" => {
            let bites = block.property("bites")?.parse::<u8>().ok()?;
            (bites <= 6).then_some((7 - bites) * 2)
        }
        "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron" => {
            let level = block.property("level")?.parse::<u8>().ok()?;
            (1..=3).contains(&level).then_some(level)
        }
        "minecraft:lava_cauldron" => Some(3),
        "minecraft:cauldron" => Some(0),
        "minecraft:composter" => {
            let level = block.property("level")?.parse::<u8>().ok()?;
            (level <= 8).then_some(level)
        }
        _ => None,
    }
}

fn comparator_side_input(
    world: &impl World,
    pos: Pos,
    block: &Block,
    comparator_output: &HashMap<Pos, u8>,
) -> u8 {
    let facing = block.property("facing").unwrap_or("north");
    let sides: [&str; 2] = if facing == "north" || facing == "south" {
        ["east", "west"]
    } else {
        ["north", "south"]
    };
    sides
        .into_iter()
        .filter_map(|direction| {
            let source = offset(pos, horizontal_delta(direction));
            world
                .block(source)
                .map(|neighbor| match neighbor.id.as_str() {
                    "minecraft:redstone_block" => 15,
                    "minecraft:redstone_wire" => neighbor
                        .property("power")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0),
                    "minecraft:comparator" => {
                        comparator_signal(&neighbor, source, direction, comparator_output)
                    }
                    _ => direct_signal_from(&neighbor, direction),
                })
        })
        .max()
        .unwrap_or(0)
}

fn comparator_should_turn_on(
    world: &impl World,
    pos: Pos,
    block: &Block,
    comparator_output: &HashMap<Pos, u8>,
    container_signal: &HashMap<Pos, u8>,
) -> bool {
    let input = comparator_input(world, pos, block, comparator_output, container_signal);
    if input == 0 {
        return false;
    }
    let side = comparator_side_input(world, pos, block, comparator_output);
    input > side || input == side && block.property("mode") != Some("subtract")
}

fn comparator_target_output(
    world: &impl World,
    pos: Pos,
    block: &Block,
    comparator_output: &HashMap<Pos, u8>,
    container_signal: &HashMap<Pos, u8>,
) -> u8 {
    let input = comparator_input(world, pos, block, comparator_output, container_signal);
    if input == 0 {
        return 0;
    }
    let side = comparator_side_input(world, pos, block, comparator_output);
    if side > input {
        0
    } else if block.property("mode") == Some("subtract") {
        input - side
    } else {
        input
    }
}

fn target_wire_power(world: &impl World, pos: Pos, comparator_output: &HashMap<Pos, u8>) -> u8 {
    let direct = HORIZONTAL
        .iter()
        .map(|&(x, y, z, direction)| (offset(pos, (x, y, z)), opposite(direction)))
        .chain([
            (offset(pos, (0, 1, 0)), "down"),
            (offset(pos, (0, -1, 0)), "up"),
        ])
        .filter(|(p, _)| {
            world
                .block(*p)
                .is_some_and(|block| block.id != "minecraft:redstone_wire")
        })
        .map(|(p, direction)| signal_at(world, p, direction, comparator_output))
        .max()
        .unwrap_or(0);
    let above_clear = !world
        .block(offset(pos, (0, 1, 0)))
        .is_some_and(|block| is_tested_conductor(&block));
    let mut incoming = 0;
    for &(x, y, z, _) in &HORIZONTAL {
        let adjacent_pos = offset(pos, (x, y, z));
        let adjacent = world.block(adjacent_pos);
        incoming = incoming.max(wire_power_at(adjacent.as_ref()));
        if adjacent.as_ref().is_some_and(is_tested_conductor) {
            if above_clear {
                incoming = incoming.max(wire_power_at(
                    world.block(offset(adjacent_pos, (0, 1, 0))).as_ref(),
                ));
            }
        } else {
            incoming = incoming.max(wire_power_at(
                world.block(offset(adjacent_pos, (0, -1, 0))).as_ref(),
            ));
        }
    }
    let wire = incoming.saturating_sub(1);
    direct.max(wire)
}

fn wire_power_at(block: Option<&Block>) -> u8 {
    block
        .filter(|block| block.id == "minecraft:redstone_wire")
        .and_then(|block| block.property("power"))
        .and_then(|power| power.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn perpendicular_wire_connects_to_comparator_but_not_repeater() {
        let comparator = Block::new("minecraft:comparator").with("facing", "north");
        let repeater = Block::new("minecraft:repeater").with("facing", "north");
        assert!(connects_to_wire(&comparator, "west"));
        assert!(!connects_to_wire(&repeater, "west"));
        assert!(connects_to_wire(&repeater, "north"));
        assert!(connects_to_wire(&repeater, "south"));
    }

    #[derive(Default)]
    struct TestWorld(BTreeMap<Pos, Block>);
    impl World for TestWorld {
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

    #[test]
    fn observed_lever_dust_lamp_transitions() {
        let mut world = TestWorld::default();
        world.set_block(
            (0, 81, 0),
            Some(Block::new("minecraft:lever").with("powered", "false")),
        );
        world.set_block(
            (1, 81, 0),
            Some(Block::new("minecraft:redstone_wire").with("power", "0")),
        );
        world.set_block(
            (2, 81, 0),
            Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
        );
        let mut engine = RedstoneEngine::default();
        engine.changed(&mut world, &[(0, 81, 0), (1, 81, 0), (2, 81, 0)], 0);
        assert_eq!(
            world.block((1, 81, 0)).unwrap().property("east"),
            Some("side")
        );
        engine.use_block(&mut world, (0, 81, 0), 2).unwrap();
        assert_eq!(
            world.block((1, 81, 0)).unwrap().property("power"),
            Some("15")
        );
        assert_eq!(
            world.block((2, 81, 0)).unwrap().property("lit"),
            Some("true")
        );
        engine.use_block(&mut world, (0, 81, 0), 5).unwrap();
        assert_eq!(
            world.block((1, 81, 0)).unwrap().property("power"),
            Some("0")
        );
        assert_eq!(
            world.block((2, 81, 0)).unwrap().property("lit"),
            Some("true")
        );
        for tick in 6..9 {
            engine.tick(&mut world, tick);
        }
        assert_eq!(
            world.block((2, 81, 0)).unwrap().property("lit"),
            Some("true")
        );
        engine.tick(&mut world, 9);
        assert_eq!(
            world.block((2, 81, 0)).unwrap().property("lit"),
            Some("false")
        );
    }

    #[test]
    fn stone_plate_requires_living_overlap_and_releases_on_scheduled_check() {
        let mut world = TestWorld::default();
        let pos = (0, 81, 0);
        world.set_block((0, 80, 0), Some(Block::new("minecraft:stone")));
        world.set_block(
            pos,
            Some(Block::new("minecraft:stone_pressure_plate").with("powered", "false")),
        );
        let mut engine = RedstoneEngine::default();
        let item = PlateEntity {
            min: [0.4, 81.1, 0.4],
            max: [0.6, 81.35, 0.6],
            living: false,
        };
        engine.update_pressure_plate_entities(&mut world, &[item], 1);
        assert_eq!(world.block(pos).unwrap().property("powered"), Some("false"));
        let player = PlateEntity {
            living: true,
            ..item
        };
        engine.update_pressure_plate_entities(&mut world, &[player], 2);
        assert_eq!(world.block(pos).unwrap().property("powered"), Some("true"));
        engine.update_pressure_plate_entities(&mut world, &[], 3);
        engine.tick(&mut world, 21);
        assert_eq!(world.block(pos).unwrap().property("powered"), Some("true"));
        engine.tick(&mut world, 22);
        assert_eq!(world.block(pos).unwrap().property("powered"), Some("false"));
    }
}
