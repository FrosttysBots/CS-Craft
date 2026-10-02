//! Redstone on the server level (26.3 `SignalGetter`, `RedstoneWireBlock`
//! with `DefaultRedstoneWireEvaluator`, `DiodeBlock`, `RepeaterBlock`,
//! `ComparatorBlock`, `RedstoneTorchBlock`, `RedstoneWallTorchBlock`,
//! `LeverBlock`, `ButtonBlock`, `RedstoneLampBlock`, `ObserverBlock`,
//! `PoweredBlock`, and the powered doors, trapdoors, fence gates and note
//! blocks). Experimental redstone (`redstone_experiments`) is off, so
//! orientations are null and draw nothing from the level random.

use super::{update, Level};
use minecraftoss_core::block::flags;
use minecraftoss_core::nbt::Tag;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{AnyRandom, RandomSource};
use minecraftoss_core::{BlockId, BlockPos, BlockStateId, Registries};
use minecraftoss_generator::feature::update::wire_connection_state;
use minecraftoss_generator::feature::{Ctx, Library, World};
use std::collections::HashMap;

/// `TickPriority`.
pub mod priority {
    pub const EXTREMELY_HIGH: i32 = -3;
    pub const VERY_HIGH: i32 = -2;
    pub const HIGH: i32 = -1;
    pub const NORMAL: i32 = 0;
}

/// The redstone behaviour of a block, by vanilla class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wire,
    Repeater,
    Comparator,
    Torch,
    WallTorch,
    Lever,
    /// `ButtonBlock` and its `ticksToStayPressed`.
    Button(i32),
    Lamp,
    Observer,
    /// `PoweredBlock` (the block of redstone).
    RedstoneBlock,
    Door,
    TrapDoor,
    FenceGate,
    NoteBlock,
    /// `PistonBaseBlock`.
    Piston { sticky: bool },
    /// `MovingPistonBlock`.
    MovingPiston,
    /// `PistonHeadBlock`.
    PistonHead,
    /// `BaseRailBlock`: straight-only (powered, activator, detector) and
    /// powered (`PoweredRailBlock`).
    Rail { straight: bool, powered: bool },
    /// `HopperBlock`.
    Hopper,
    /// Other simulated item containers (chests, barrels, shulker boxes,
    /// dispensers and droppers).
    Container,
    /// `DispenserBlock` and `DropperBlock`.
    Dispenser { dropper: bool },
    /// `CopperBulbBlock`.
    CopperBulb,
    /// `DaylightDetectorBlock`.
    DaylightDetector,
    /// `BasePressurePlateBlock`.
    PressurePlate,
    /// `LightningRodBlock`.
    LightningRod,
}

/// Block ids by redstone behaviour.
pub struct Kinds {
    by_block: HashMap<BlockId, Kind>,
    pub wire: BlockId,
    pub comparator: BlockId,
    pub redstone_block: BlockId,
}

impl Kinds {
    pub fn new(registries: &Registries) -> Self {
        // Most derived class first.
        let classes: &[(&str, Kind)] = &[
            ("RedstoneWireBlock", Kind::Wire),
            ("RepeaterBlock", Kind::Repeater),
            ("ComparatorBlock", Kind::Comparator),
            ("RedstoneWallTorchBlock", Kind::WallTorch),
            ("RedstoneTorchBlock", Kind::Torch),
            ("LeverBlock", Kind::Lever),
            ("ButtonBlock", Kind::Button(0)),
            ("RedstoneLampBlock", Kind::Lamp),
            ("ObserverBlock", Kind::Observer),
            ("PoweredBlock", Kind::RedstoneBlock),
            ("DoorBlock", Kind::Door),
            ("TrapDoorBlock", Kind::TrapDoor),
            ("FenceGateBlock", Kind::FenceGate),
            ("NoteBlock", Kind::NoteBlock),
            ("PistonBaseBlock", Kind::Piston { sticky: false }),
            ("MovingPistonBlock", Kind::MovingPiston),
            ("PistonHeadBlock", Kind::PistonHead),
            ("PoweredRailBlock", Kind::Rail { straight: true, powered: true }),
            ("DetectorRailBlock", Kind::Rail { straight: true, powered: false }),
            ("RailBlock", Kind::Rail { straight: false, powered: false }),
            ("HopperBlock", Kind::Hopper),
            ("ChestBlock", Kind::Container),
            ("BarrelBlock", Kind::Container),
            ("ShulkerBoxBlock", Kind::Container),
            ("DropperBlock", Kind::Dispenser { dropper: true }),
            ("DispenserBlock", Kind::Dispenser { dropper: false }),
            ("CopperBulbBlock", Kind::CopperBulb),
            ("DaylightDetectorBlock", Kind::DaylightDetector),
            ("BasePressurePlateBlock", Kind::PressurePlate),
            ("LightningRodBlock", Kind::LightningRod),
        ];
        let mut by_block = HashMap::new();
        for (id, info) in registries.blocks.blocks() {
            let Some(mut kind) = classes.iter().find(|(class, _)| info.is_a(class)).map(|&(_, k)| k) else { continue };
            if kind == Kind::Button(0) {
                // Stone buttons stay pressed for 20 ticks, wooden ones 30.
                let stone = matches!(info.name.as_str(), "minecraft:stone_button" | "minecraft:polished_blackstone_button");
                kind = Kind::Button(if stone { 20 } else { 30 });
            }
            if kind == (Kind::Piston { sticky: false }) {
                kind = Kind::Piston { sticky: info.name.as_str() == "minecraft:sticky_piston" };
            }
            by_block.insert(id, kind);
        }
        let id = |name: &str| registries.blocks.block_by_name(name).expect("vanilla block");
        Self { by_block, wire: id("minecraft:redstone_wire"), comparator: id("minecraft:comparator"), redstone_block: id("minecraft:redstone_block") }
    }

    pub fn get(&self, block: BlockId) -> Option<Kind> {
        self.by_block.get(&block).copied()
    }
}

/// A read-only `World` over the level, for the generator's shape helpers
/// that take a `Ctx`.
struct View<'l, 'a> {
    level: &'l Level<'a>,
    random: AnyRandom,
}

impl World for View<'_, '_> {
    fn block_at(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.level.block(BlockPos::new(x, y, z))
    }

    fn set_block_with_flags(&mut self, _lib: &Library, _pos: BlockPos, _state: BlockStateId, _flags: u32) -> bool {
        unreachable!("the redstone view is read-only")
    }

    fn schedule(&mut self, _pos: BlockPos, _fluid: bool, _delay: i32) {}

    fn random(&mut self) -> &mut AnyRandom {
        &mut self.random
    }

    fn world_seed(&self) -> i64 {
        World::world_seed(self.level)
    }

    fn light(&self, pos: BlockPos, darkening: i32) -> Option<(i32, bool)> {
        World::light(self.level, pos, darkening)
    }

    fn min_y(&self) -> i32 {
        World::min_y(self.level)
    }

    fn max_y(&self) -> i32 {
        World::max_y(self.level)
    }

    fn height_at(&self, kind: minecraftoss_core::chunk::HeightmapKind, x: i32, z: i32) -> i32 {
        World::height_at(self.level, kind, x, z)
    }
}

/// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`: towards
/// the support.
fn connected_direction(face: Option<&str>, facing: Direction) -> Direction {
    match face {
        Some("ceiling") => Direction::Down,
        Some("floor") => Direction::Up,
        _ => facing,
    }
}

impl Level<'_> {
    pub(super) fn redstone_kind(&self, state: BlockStateId) -> Option<Kind> {
        self.kinds.get(self.block_id(state))
    }

    fn prop(&self, state: BlockStateId, name: &str) -> Option<&str> {
        self.registries().blocks.property(state, name)
    }

    fn flag(&self, state: BlockStateId, name: &str) -> bool {
        self.prop(state, name) == Some("true")
    }

    fn int_prop(&self, state: BlockStateId, name: &str) -> i32 {
        self.prop(state, name).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    fn facing(&self, state: BlockStateId) -> Direction {
        self.prop(state, "facing").and_then(Direction::from_name).unwrap_or(Direction::North)
    }

    pub(super) fn with(&self, state: BlockStateId, name: &str, value: &str) -> BlockStateId {
        self.registries().blocks.with_property(state, name, value).unwrap_or(state)
    }

    fn with_bool(&self, state: BlockStateId, name: &str, value: bool) -> BlockStateId {
        self.with(state, name, if value { "true" } else { "false" })
    }

    fn is_conductor(&self, state: BlockStateId) -> bool {
        self.registries().blocks.is(state, flags::REDSTONE_CONDUCTOR)
    }

    pub(super) fn can_survive_state(&self, state: BlockStateId, pos: BlockPos) -> bool {
        self.lib.survival.can_survive(&self.lib.registries, self, state, (pos.x, pos.y, pos.z))
    }

    fn view(&self) -> View<'_, '_> {
        View { level: self, random: AnyRandom::new(true, 0) }
    }

    // ---- signals (`SignalGetter`, `BlockBehaviour`) -------------------------

    /// `BlockState.isSignalSource`.
    pub fn is_signal_source(&self, state: BlockStateId) -> bool {
        match self.redstone_kind(state) {
            Some(Kind::Wire) => self.wire_signals.get(),
            _ => self.registries().blocks.is(state, flags::SIGNAL_SOURCE),
        }
    }

    /// `ComparatorBlockEntity.getOutputSignal`.
    pub fn comparator_output_at(&self, pos: BlockPos) -> i32 {
        let chunk = self.chunk(pos.chunk());
        let tag = chunk.and_then(|c| c.block_entities.entities.get(&(pos.x, pos.y, pos.z)));
        tag.and_then(|t| t.get("OutputSignal")).and_then(Tag::as_i64).unwrap_or(0) as i32
    }

    fn set_comparator_output(&mut self, pos: BlockPos, value: i32) {
        let key = (pos.x, pos.y, pos.z);
        if let Some(Tag::Compound(map)) = self.chunks.get_mut(&pos.chunk()).and_then(|c| c.block_entities.entities.get_mut(&key)) {
            map.insert("OutputSignal".to_owned(), Tag::Int(value));
        }
    }

    /// `BlockState.getOwnSignal`.
    fn own_signal(&self, state: BlockStateId, pos: BlockPos) -> i32 {
        match self.redstone_kind(state) {
            Some(Kind::Wire) => self.int_prop(state, "power"),
            Some(Kind::Repeater) => {
                if self.flag(state, "powered") {
                    15
                } else {
                    0
                }
            }
            Some(Kind::Comparator) => {
                if self.flag(state, "powered") {
                    self.comparator_output_at(pos)
                } else {
                    0
                }
            }
            Some(Kind::Torch | Kind::WallTorch) => {
                if self.flag(state, "lit") {
                    15
                } else {
                    0
                }
            }
            Some(Kind::Lever | Kind::Button(_) | Kind::Observer | Kind::LightningRod) => {
                if self.flag(state, "powered") {
                    15
                } else {
                    0
                }
            }
            Some(Kind::RedstoneBlock) => 15,
            Some(Kind::DaylightDetector) => self.int_prop(state, "power"),
            Some(Kind::PressurePlate) => self.plate_signal(state),
            _ => 0,
        }
    }

    /// `BlockState.getSignal(level, pos, direction)`: `direction` points from
    /// the reader to this block.
    pub fn state_signal(&self, state: BlockStateId, pos: BlockPos, direction: Direction) -> i32 {
        match self.redstone_kind(state) {
            Some(Kind::Wire) => {
                if !self.wire_signals.get() || direction == Direction::Down {
                    return 0;
                }
                let power = self.own_signal(state, pos);
                if power == 0 {
                    return 0;
                }
                if direction != Direction::Up {
                    let mut view = self.view();
                    let connected = wire_connection_state(&Ctx { lib: self.lib, region: &mut view }, state, pos);
                    if self.prop(connected, direction.opposite().name()) == Some("none") {
                        return 0;
                    }
                }
                power
            }
            Some(Kind::Repeater | Kind::Comparator | Kind::Observer) => {
                if self.facing(state) == direction {
                    self.own_signal(state, pos)
                } else {
                    0
                }
            }
            Some(Kind::Torch) => {
                if direction != Direction::Up {
                    self.own_signal(state, pos)
                } else {
                    0
                }
            }
            Some(Kind::WallTorch) => {
                if self.facing(state) != direction {
                    self.own_signal(state, pos)
                } else {
                    0
                }
            }
            _ => self.own_signal(state, pos),
        }
    }

    /// `BlockState.getDirectSignal`.
    fn state_direct_signal(&self, state: BlockStateId, pos: BlockPos, direction: Direction) -> i32 {
        match self.redstone_kind(state) {
            Some(Kind::Wire | Kind::Repeater | Kind::Comparator | Kind::Observer) => self.state_signal(state, pos, direction),
            Some(Kind::Torch | Kind::WallTorch) => {
                if direction == Direction::Down {
                    self.state_signal(state, pos, direction)
                } else {
                    0
                }
            }
            Some(Kind::LightningRod) => {
                if self.flag(state, "powered") && self.prop(state, "facing").and_then(Direction::from_name) == Some(direction) {
                    15
                } else {
                    0
                }
            }
            Some(Kind::PressurePlate) => {
                if direction == Direction::Up {
                    self.plate_signal(state)
                } else {
                    0
                }
            }
            Some(Kind::Lever | Kind::Button(_)) => {
                let towards = connected_direction(self.prop(state, "face"), self.facing(state));
                if self.flag(state, "powered") && towards == direction {
                    15
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    /// `SignalGetter.getDirectSignal(pos, direction)`.
    fn direct_signal(&self, pos: BlockPos, direction: Direction) -> i32 {
        self.state_direct_signal(self.block(pos), pos, direction)
    }

    /// `SignalGetter.getDirectSignalTo`: strong power into a block.
    fn direct_signal_to(&self, pos: BlockPos) -> i32 {
        let mut result = 0;
        for direction in [Direction::Down, Direction::Up, Direction::North, Direction::South, Direction::West, Direction::East] {
            result = result.max(self.direct_signal(pos.relative(direction, 1), direction));
            if result >= 15 {
                return result;
            }
        }
        result
    }

    /// `SignalGetter.getSignal(pos, direction)`.
    pub fn signal(&self, pos: BlockPos, direction: Direction) -> i32 {
        let state = self.block(pos);
        let signal = self.state_signal(state, pos, direction);
        if self.is_conductor(state) {
            signal.max(self.direct_signal_to(pos))
        } else {
            signal
        }
    }

    fn has_signal(&self, pos: BlockPos, direction: Direction) -> bool {
        self.signal(pos, direction) > 0
    }

    /// `SignalGetter.hasNeighborSignal`.
    pub fn has_neighbor_signal(&self, pos: BlockPos) -> bool {
        [Direction::Down, Direction::Up, Direction::North, Direction::South, Direction::West, Direction::East]
            .into_iter()
            .any(|d| self.signal(pos.relative(d, 1), d) > 0)
    }

    /// `SignalGetter.getBestNeighborSignal`.
    pub fn best_neighbor_signal(&self, pos: BlockPos) -> i32 {
        let mut best = 0;
        for direction in Direction::ALL {
            let signal = self.signal(pos.relative(direction, 1), direction);
            if signal >= 15 {
                return 15;
            }
            best = best.max(signal);
        }
        best
    }

    /// `SignalGetter.getControlInputSignal`.
    fn control_input_signal(&self, pos: BlockPos, direction: Direction, only_diodes: bool) -> i32 {
        let state = self.block(pos);
        let kind = self.redstone_kind(state);
        if only_diodes {
            if matches!(kind, Some(Kind::Repeater | Kind::Comparator)) {
                self.direct_signal(pos, direction)
            } else {
                0
            }
        } else if kind == Some(Kind::RedstoneBlock) {
            15
        } else if kind == Some(Kind::Wire) {
            self.int_prop(state, "power")
        } else if self.is_signal_source(state) {
            self.direct_signal(pos, direction)
        } else {
            0
        }
    }

    /// `BlockState.hasAnalogOutputSignal` with `getAnalogOutputSignal`, for
    /// the blocks whose output follows their state.
    fn analog_output(&self, state: BlockStateId, pos: BlockPos) -> Option<i32> {
        if let Some(store) = self.store_of(state) {
            let container = if store == super::container::Store::Chest {
                self.container_at(pos, false)
            } else {
                self.container_at(pos, true)
            };
            return Some(self.container_signal(container));
        }
        self.state_analog_output(state)
    }

    /// `getAnalogOutputSignal` of blocks whose output follows their state.
    fn state_analog_output(&self, state: BlockStateId) -> Option<i32> {
        let blocks = &self.registries().blocks;
        let info = blocks.block(blocks.block_of(state));
        Some(match info.name.as_str() {
            "minecraft:cake" => (7 - self.int_prop(state, "bites")) * 2,
            "minecraft:end_portal_frame" => {
                if self.flag(state, "eye") {
                    15
                } else {
                    0
                }
            }
            "minecraft:composter" => self.int_prop(state, "level"),
            "minecraft:respawn_anchor" => (self.int_prop(state, "charges") as f32 / 4.0 * 15.0).floor() as i32,
            "minecraft:cauldron" => 0,
            "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron" => self.int_prop(state, "level"),
            "minecraft:lava_cauldron" => 3,
            _ if info.is_a("CopperBulbBlock") => {
                if self.flag(state, "lit") {
                    15
                } else {
                    0
                }
            }
            _ if info.is_a("CandleCakeBlock") => 14,
            _ => return None,
        })
    }

    // ---- neighbour notification helpers ------------------------------------

    /// `Level.neighborChanged(pos, block, orientation)`.
    fn neighbor_changed_at(&mut self, pos: BlockPos, source: BlockId) {
        self.add_and_run(super::Update::Simple { pos, block: source });
    }

    /// `Level.updateNeighborsAtExceptFromFacing`.
    fn update_neighbors_except(&mut self, pos: BlockPos, source: BlockId, skip: Direction) {
        self.add_and_run(super::Update::Neighbors { source: pos, block: source, skip: Some(skip), index: 0 });
    }

    /// `Level.updateNeighbourForOutputSignal`: comparators beside the block,
    /// or behind a conductor beside it.
    pub(super) fn update_neighbour_for_output_signal(&mut self, pos: BlockPos, changed: BlockId) {
        for direction in Direction::HORIZONTAL {
            let mut relative = pos.relative(direction, 1);
            if self.chunk(relative.chunk()).is_none() {
                continue;
            }
            let mut state = self.block(relative);
            if self.block_id(state) == self.kinds.comparator {
                self.add_and_run(super::Update::Full { state, pos: relative, block: changed });
            } else if self.is_conductor(state) {
                relative = relative.relative(direction, 1);
                state = self.block(relative);
                if self.block_id(state) == self.kinds.comparator {
                    self.add_and_run(super::Update::Full { state, pos: relative, block: changed });
                }
            }
        }
    }

    pub(super) fn has_analog_output(&self, state: BlockStateId) -> bool {
        self.store_of(state).is_some() || self.state_analog_output(state).is_some()
    }

    // ---- block hooks --------------------------------------------------------

    /// `onPlace` for redstone blocks.
    pub(super) fn redstone_on_place(&mut self, kind: Kind, state: BlockStateId, pos: BlockPos, old: BlockStateId) {
        let block = self.block_id(state);
        match kind {
            Kind::Wire => {
                if self.block_id(old) != block {
                    self.wire_update_power(pos, state);
                    for direction in Direction::VERTICAL {
                        self.update_neighbors_at(pos.relative(direction, 1), block);
                    }
                    self.wire_update_neighbors_of_neighboring_wires(pos);
                }
            }
            Kind::Repeater | Kind::Comparator => self.diode_update_in_front(pos, state),
            Kind::Torch | Kind::WallTorch => self.torch_notify(pos, block),
            Kind::Rail { .. } => self.rail_on_place(state, pos, old, false),
            Kind::Hopper => {
                if self.block_id(old) != block {
                    self.hopper_check_powered(pos, state);
                }
            }
            Kind::CopperBulb => {
                if self.block_id(old) != block {
                    self.copper_bulb_check(state, pos);
                }
            }
            Kind::LightningRod => {
                if self.block_id(old) != block && self.flag(state, "powered") && !self.has_block_tick_at(pos, block) {
                    self.schedule_block_tick_priority(pos, block, 8, priority::NORMAL);
                }
            }
            Kind::Piston { .. } => {
                if self.block_id(old) != block && !self.moving.entities.contains_key(&pos) {
                    self.piston_check_if_extend(pos, state);
                }
            }
            Kind::Observer => {
                if self.block_id(old) != block && self.flag(state, "powered") && !self.has_block_tick_at(pos, block) {
                    let off = self.with_bool(state, "powered", false);
                    self.set_block(pos, off, update::CLIENTS | update::KNOWN_SHAPE, update::LIMIT);
                    self.observer_update_in_front(pos, off);
                }
            }
            _ => {}
        }
    }

    /// `affectNeighborsAfterRemoval` for redstone blocks.
    pub(super) fn redstone_after_removal(&mut self, kind: Kind, state: BlockStateId, pos: BlockPos, moved: bool) {
        let block = self.block_id(state);
        match kind {
            Kind::Wire if !moved => {
                for direction in Direction::ALL {
                    self.update_neighbors_at(pos.relative(direction, 1), block);
                }
                self.wire_update_power(pos, state);
                self.wire_update_neighbors_of_neighboring_wires(pos);
            }
            Kind::Repeater | Kind::Comparator if !moved => self.diode_update_in_front(pos, state),
            Kind::Torch | Kind::WallTorch if !moved => self.torch_notify(pos, block),
            Kind::Lever | Kind::Button(_) if !moved && self.flag(state, "powered") => self.attached_update_neighbours(pos, state),
            Kind::PistonHead => self.piston_head_after_removal(state, pos),
            Kind::Rail { .. } => self.rail_after_removal(state, pos, moved),
            Kind::LightningRod if self.flag(state, "powered") => self.lightning_rod_update(state, pos),
            Kind::PressurePlate if !moved && self.plate_signal(state) > 0 => {
                self.update_neighbors_at(pos, block);
                self.update_neighbors_at(pos.below(), block);
            }
            // `Containers.updateNeighboursAfterDestroy`.
            Kind::Hopper | Kind::Container | Kind::Dispenser { .. } => self.update_neighbour_for_output_signal(pos, block),
            Kind::Observer => {
                if self.flag(state, "powered") && self.has_block_tick_at(pos, block) {
                    let off = self.with_bool(state, "powered", false);
                    self.observer_update_in_front(pos, off);
                }
            }
            _ => {}
        }
    }

    /// `neighborChanged` for redstone blocks.
    pub(super) fn redstone_neighbor_changed(&mut self, kind: Kind, state: BlockStateId, pos: BlockPos, source: BlockId) {
        match kind {
            Kind::Piston { .. } => self.piston_check_if_extend(pos, state),
            Kind::PistonHead => self.piston_head_neighbor_changed(state, pos, source),
            Kind::Rail { .. } => self.rail_neighbor_changed(state, pos, source),
            Kind::Hopper => self.hopper_check_powered(pos, state),
            Kind::Dispenser { .. } => self.dispenser_neighbor_changed(state, pos),
            Kind::CopperBulb => self.copper_bulb_check(state, pos),
            Kind::Wire => {
                if self.can_survive_state(state, pos) {
                    self.wire_update_power(pos, state);
                } else {
                    self.drop_resources(state, pos);
                    self.remove_block(pos, false);
                }
            }
            Kind::Repeater | Kind::Comparator => {
                let block = self.block_id(state);
                if self.block_id(self.block(pos)) != block {
                    return;
                }
                if self.can_survive_state(state, pos) {
                    if kind == Kind::Comparator {
                        self.comparator_check_tick(pos, state);
                    } else {
                        self.diode_check_tick(pos, state);
                    }
                } else {
                    self.drop_resources(state, pos);
                    self.remove_block(pos, false);
                    for direction in Direction::ALL {
                        self.update_neighbors_at(pos.relative(direction, 1), block);
                    }
                }
            }
            Kind::Torch | Kind::WallTorch => {
                let block = self.block_id(state);
                if self.flag(state, "lit") == self.torch_input(kind, state, pos) && !self.will_tick_this_tick(pos, block) {
                    self.schedule_block_tick_priority(pos, block, 2, priority::NORMAL);
                }
            }
            Kind::Lamp => {
                let lit = self.flag(state, "lit");
                if lit != self.has_neighbor_signal(pos) {
                    if lit {
                        let block = self.block_id(state);
                        self.schedule_block_tick_priority(pos, block, 4, priority::NORMAL);
                    } else {
                        let on = self.with_bool(state, "lit", true);
                        self.set_block(pos, on, update::CLIENTS, update::LIMIT);
                    }
                }
            }
            Kind::Door => {
                let other = if self.prop(state, "half") == Some("lower") { Direction::Up } else { Direction::Down };
                let signal = self.has_neighbor_signal(pos) || self.has_neighbor_signal(pos.relative(other, 1));
                // `!defaultBlockState().is(block)`: another door's own update is ignored.
                if source != self.block_id(state) && signal != self.flag(state, "powered") {
                    if signal != self.flag(state, "open") {
                        self.random.next_f32();
                    }
                    let next = self.with_bool(self.with_bool(state, "powered", signal), "open", signal);
                    self.set_block(pos, next, update::CLIENTS, update::LIMIT);
                }
            }
            Kind::TrapDoor => {
                let signal = self.has_neighbor_signal(pos);
                if signal != self.flag(state, "powered") {
                    let mut next = state;
                    if self.flag(state, "open") != signal {
                        next = self.with_bool(next, "open", signal);
                        self.random.next_f32();
                    }
                    let next = self.with_bool(next, "powered", signal);
                    self.set_block(pos, next, update::CLIENTS, update::LIMIT);
                    if self.flag(next, "waterlogged") {
                        self.schedule_water_tick(pos);
                    }
                }
            }
            Kind::FenceGate => {
                let signal = self.has_neighbor_signal(pos);
                if self.flag(state, "powered") != signal {
                    let next = self.with_bool(self.with_bool(state, "powered", signal), "open", signal);
                    self.set_block(pos, next, update::CLIENTS, update::LIMIT);
                    if self.flag(state, "open") != signal {
                        self.random.next_f32();
                    }
                }
            }
            Kind::NoteBlock => {
                let signal = self.has_neighbor_signal(pos);
                if signal != self.flag(state, "powered") {
                    if signal {
                        self.play_note(state, pos);
                    }
                    let next = self.with_bool(state, "powered", signal);
                    self.set_block_and_update(pos, next);
                }
            }
            _ => {}
        }
    }

    /// `tick` for redstone blocks.
    pub(super) fn redstone_tick(&mut self, kind: Kind, state: BlockStateId, pos: BlockPos) {
        let block = self.block_id(state);
        match kind {
            Kind::Repeater => {
                if self.repeater_locked(pos, state) {
                    return;
                }
                let on = self.flag(state, "powered");
                let should = self.diode_should_turn_on(kind, pos, state);
                if on && !should {
                    self.set_block(pos, self.with_bool(state, "powered", false), update::CLIENTS, update::LIMIT);
                } else if !on {
                    self.set_block(pos, self.with_bool(state, "powered", true), update::CLIENTS, update::LIMIT);
                    if !should {
                        let delay = self.int_prop(state, "delay") * 2;
                        self.schedule_block_tick_priority(pos, block, delay, priority::VERY_HIGH);
                    }
                }
            }
            Kind::Comparator => self.comparator_refresh(pos, state),
            Kind::LightningRod => {
                self.set_block_and_update(pos, self.with_bool(state, "powered", false));
                self.lightning_rod_update(state, pos);
            }
            Kind::PressurePlate => {
                let signal = self.plate_signal(state);
                if signal > 0 {
                    self.plate_check_pressed(pos, state, signal);
                }
            }
            Kind::Dispenser { dropper } => self.dispense_from(pos, dropper),
            Kind::Torch | Kind::WallTorch => {
                let signal = self.torch_input(kind, state, pos);
                let now = self.game_time;
                while self.torch_toggles.first().is_some_and(|&(_, when)| now - when > 60) {
                    self.torch_toggles.remove(0);
                }
                if self.flag(state, "lit") {
                    if signal {
                        self.set_block_and_update(pos, self.with_bool(state, "lit", false));
                        if self.torch_toggled_too_often(pos, true) {
                            let now_block = self.block_id(self.block(pos));
                            self.schedule_block_tick_priority(pos, now_block, 160, priority::NORMAL);
                        }
                    }
                } else if !signal && !self.torch_toggled_too_often(pos, false) {
                    self.set_block_and_update(pos, self.with_bool(state, "lit", true));
                }
            }
            Kind::Lamp => {
                if self.flag(state, "lit") && !self.has_neighbor_signal(pos) {
                    self.set_block(pos, self.with_bool(state, "lit", false), update::CLIENTS, update::LIMIT);
                }
            }
            Kind::Button(_) => {
                // `checkPressed` without arrows: the button releases.
                if self.flag(state, "powered") {
                    self.set_block_and_update(pos, self.with_bool(state, "powered", false));
                    self.attached_update_neighbours(pos, state);
                }
            }
            Kind::Observer => {
                if self.flag(state, "powered") {
                    self.set_block(pos, self.with_bool(state, "powered", false), update::CLIENTS, update::LIMIT);
                } else {
                    self.set_block(pos, self.with_bool(state, "powered", true), update::CLIENTS, update::LIMIT);
                    self.schedule_block_tick_priority(pos, block, 2, priority::NORMAL);
                }
                self.observer_update_in_front(pos, state);
            }
            _ => {}
        }
    }

    /// `RedstoneWireBlock.updateIndirectNeighbourShapes`: wires one step
    /// down or up diagonally re-check their connection to this side.
    pub(super) fn wire_update_indirect_shapes(&mut self, state: BlockStateId, pos: BlockPos, flags: u32, limit: i32) {
        for direction in Direction::HORIZONTAL {
            if self.prop(state, direction.name()).is_none_or(|v| v == "none") {
                continue;
            }
            let side = pos.relative(direction, 1);
            if self.block_id(self.block(side)) == self.kinds.wire {
                continue;
            }
            for dy in [Direction::Down, Direction::Up] {
                let wire_pos = side.relative(dy, 1);
                if self.block_id(self.block(wire_pos)) == self.kinds.wire {
                    let neighbor_pos = wire_pos.relative(direction.opposite(), 1);
                    let neighbor_state = self.block(neighbor_pos);
                    self.add_and_run(super::Update::Shape {
                        direction: direction.opposite(),
                        neighbor_state,
                        pos: wire_pos,
                        neighbor_pos,
                        flags,
                        limit,
                    });
                }
            }
        }
    }

    // ---- lever and button -----------------------------------------------------

    /// `LeverBlock.pull`: the lever flips and updates around it and its support.
    pub fn pull_lever(&mut self, pos: BlockPos) {
        let state = self.block(pos);
        if self.redstone_kind(state) != Some(Kind::Lever) {
            return;
        }
        let next = self.with_bool(state, "powered", !self.flag(state, "powered"));
        self.set_block_and_update(pos, next);
        self.attached_update_neighbours(pos, next);
    }

    /// `ButtonBlock.press`.
    pub fn press_button(&mut self, pos: BlockPos) {
        let state = self.block(pos);
        let Some(Kind::Button(ticks)) = self.redstone_kind(state) else { return };
        if self.flag(state, "powered") {
            return;
        }
        self.set_block_and_update(pos, self.with_bool(state, "powered", true));
        self.attached_update_neighbours(pos, state);
        let block = self.block_id(state);
        self.schedule_block_tick_priority(pos, block, ticks, priority::NORMAL);
    }

    /// `useWithoutItem` for the redstone blocks a player can use with an
    /// empty hand (creative abilities). False for blocks not simulated.
    pub fn use_block(&mut self, pos: BlockPos) -> bool {
        self.use_block_facing(pos, None)
    }

    /// `useWithoutItem` by a player facing a horizontal direction (fence
    /// gates open away from the player).
    /// Whether `use_block_facing` acts on a block in this state (with a
    /// player facing given): the state alone decides it.
    pub fn handles_use(&self, state: BlockStateId) -> bool {
        let iron = matches!(self.name(state), "minecraft:iron_door" | "minecraft:iron_trapdoor");
        match self.redstone_kind(state) {
            Some(Kind::Door) | Some(Kind::TrapDoor) => !iron,
            Some(Kind::FenceGate | Kind::NoteBlock | Kind::Lever | Kind::Button(_) | Kind::Repeater | Kind::DaylightDetector | Kind::Comparator) => true,
            _ => false,
        }
    }

    /// Whether `attack_block` acts on a block in this state.
    pub fn handles_attack(&self, state: BlockStateId) -> bool {
        self.redstone_kind(state) == Some(Kind::NoteBlock)
    }

    pub fn use_block_facing(&mut self, pos: BlockPos, player_facing: Option<Direction>) -> bool {
        let state = self.block(pos);
        let iron = matches!(self.name(state), "minecraft:iron_door" | "minecraft:iron_trapdoor");
        match self.redstone_kind(state) {
            Some(Kind::Door) if !iron => {
                let next = self.with_bool(state, "open", !self.flag(state, "open"));
                self.set_block(pos, next, update::CLIENTS | 8, update::LIMIT);
                self.random.next_f32();
            }
            Some(Kind::TrapDoor) if !iron => {
                let next = self.with_bool(state, "open", !self.flag(state, "open"));
                self.set_block(pos, next, update::CLIENTS, update::LIMIT);
                if self.flag(next, "waterlogged") {
                    self.schedule_water_tick(pos);
                }
                self.random.next_f32();
            }
            Some(Kind::FenceGate) => {
                let next = if self.flag(state, "open") {
                    self.with_bool(state, "open", false)
                } else {
                    let Some(direction) = player_facing else { return false };
                    let mut next = state;
                    if self.facing(state) == direction.opposite() {
                        next = self.with(next, "facing", direction.name());
                    }
                    self.with_bool(next, "open", true)
                };
                self.set_block(pos, next, update::CLIENTS | 8, update::LIMIT);
                self.random.next_f32();
            }
            Some(Kind::NoteBlock) => {
                let note = (self.int_prop(state, "note") + 1) % 25;
                let next = self.with(state, "note", &note.to_string());
                self.set_block_and_update(pos, next);
                self.play_note(next, pos);
            }
            Some(Kind::Lever) => self.pull_lever(pos),
            Some(Kind::Button(_)) => self.press_button(pos),
            Some(Kind::Repeater) => {
                let delay = self.int_prop(state, "delay") % 4 + 1;
                let next = self.with(state, "delay", &delay.to_string());
                self.set_block_and_update(pos, next);
            }
            Some(Kind::DaylightDetector) => self.daylight_use(pos),
            Some(Kind::Comparator) => {
                let mode = if self.prop(state, "mode") == Some("compare") { "subtract" } else { "compare" };
                let next = self.with(state, "mode", mode);
                self.set_block(pos, next, update::CLIENTS, update::LIMIT);
                if self.block_id(self.block(pos)) == self.kinds.comparator {
                    self.comparator_refresh(pos, next);
                }
            }
            _ => return false,
        }
        true
    }

    /// `LightningRodBlock.onLightningStrike`.
    pub(super) fn lightning_rod_strike(&mut self, state: BlockStateId, pos: BlockPos) {
        self.set_block_and_update(pos, self.with_bool(state, "powered", true));
        self.lightning_rod_update(state, pos);
        let block = self.block_id(state);
        self.schedule_block_tick_priority(pos, block, 8, priority::NORMAL);
    }

    /// `LightningRodBlock.updateNeighbours`: around the block behind it.
    fn lightning_rod_update(&mut self, state: BlockStateId, pos: BlockPos) {
        let front = self.facing(state).opposite();
        let block = self.block_id(state);
        self.update_neighbors_at(pos.relative(front, 1), block);
    }

    /// `CopperBulbBlock.checkAndFlip`.
    fn copper_bulb_check(&mut self, state: BlockStateId, pos: BlockPos) {
        let signal = self.has_neighbor_signal(pos);
        if signal != self.flag(state, "powered") {
            let mut next = state;
            if !self.flag(state, "powered") {
                next = self.with_bool(next, "lit", !self.flag(state, "lit"));
            }
            self.set_block_and_update(pos, self.with_bool(next, "powered", signal));
        }
    }

    /// `BlockBehaviour.attack` by a player: note blocks play.
    pub fn attack_block(&mut self, pos: BlockPos) -> bool {
        let state = self.block(pos);
        if self.redstone_kind(state) != Some(Kind::NoteBlock) {
            return false;
        }
        self.play_note(state, pos);
        true
    }

    /// `NoteBlock.playNote`: heads play anywhere, other instruments need
    /// air above.
    fn play_note(&mut self, state: BlockStateId, pos: BlockPos) {
        let instrument = self.registries().blocks.state(state).instrument;
        let head = instrument >= minecraftoss_core::block::FIRST_HEAD_INSTRUMENT;
        if head || self.registries().blocks.is_air(self.block(pos.above())) {
            let block = self.block_id(state);
            self.block_event(pos, block, 0, 0);
        }
    }

    /// `NoteBlock.triggerEvent`: the seeded sound draws from the level random.
    pub(super) fn note_block_event(&mut self, state: BlockStateId) {
        let instrument = self.registries().blocks.state(state).instrument;
        let custom = usize::from(instrument) == minecraftoss_core::block::INSTRUMENTS.len() - 1;
        if custom {
            // A player head's `note_block_sound`; heads are not simulated.
            self.unsupported.push("note block custom head sound".to_owned());
            return;
        }
        self.random.next_i64();
    }

    /// `LeverBlock` / `ButtonBlock.updateNeighbours`.
    fn attached_update_neighbours(&mut self, pos: BlockPos, state: BlockStateId) {
        let block = self.block_id(state);
        let front = connected_direction(self.prop(state, "face"), self.facing(state)).opposite();
        self.update_neighbors_at(pos, block);
        self.update_neighbors_at(pos.relative(front, 1), block);
    }

    // ---- wire -----------------------------------------------------------------

    /// `DefaultRedstoneWireEvaluator.updatePowerStrength`.
    fn wire_update_power(&mut self, pos: BlockPos, state: BlockStateId) {
        let target = self.wire_target_strength(pos);
        if self.int_prop(state, "power") == target {
            return;
        }
        if self.block(pos) == state {
            let next = self.with(state, "power", &target.to_string());
            self.set_block(pos, next, update::CLIENTS, update::LIMIT);
        }
        let wire = self.kinds.wire;
        for p in java_hash_set_order(pos) {
            self.update_neighbors_at(p, wire);
        }
    }

    /// `DefaultRedstoneWireEvaluator.calculateTargetStrength`.
    fn wire_target_strength(&self, pos: BlockPos) -> i32 {
        self.wire_signals.set(false);
        let block_signal = self.best_neighbor_signal(pos);
        self.wire_signals.set(true);
        if block_signal == 15 {
            return 15;
        }
        block_signal.max(self.incoming_wire_signal(pos))
    }

    fn wire_power(&self, state: BlockStateId) -> i32 {
        if self.block_id(state) == self.kinds.wire {
            self.int_prop(state, "power")
        } else {
            0
        }
    }

    /// `RedstoneWireEvaluator.getIncomingWireSignal`.
    fn incoming_wire_signal(&self, pos: BlockPos) -> i32 {
        let mut signal = 0;
        for direction in Direction::HORIZONTAL {
            let neighbor = pos.relative(direction, 1);
            let state = self.block(neighbor);
            signal = signal.max(self.wire_power(state));
            let conductor = self.is_conductor(state);
            if conductor && !self.is_conductor(self.block(pos.above())) {
                signal = signal.max(self.wire_power(self.block(neighbor.above())));
            } else if !conductor {
                signal = signal.max(self.wire_power(self.block(neighbor.below())));
            }
        }
        (signal - 1).max(0)
    }

    /// `RedstoneWireBlock.checkCornerChangeAt`.
    fn wire_check_corner(&mut self, pos: BlockPos) {
        if self.block_id(self.block(pos)) != self.kinds.wire {
            return;
        }
        let wire = self.kinds.wire;
        self.update_neighbors_at(pos, wire);
        for direction in Direction::ALL {
            self.update_neighbors_at(pos.relative(direction, 1), wire);
        }
    }

    /// `RedstoneWireBlock.updateNeighborsOfNeighboringWires`.
    fn wire_update_neighbors_of_neighboring_wires(&mut self, pos: BlockPos) {
        for direction in Direction::HORIZONTAL {
            self.wire_check_corner(pos.relative(direction, 1));
        }
        for direction in Direction::HORIZONTAL {
            let target = pos.relative(direction, 1);
            if self.is_conductor(self.block(target)) {
                self.wire_check_corner(target.above());
            } else {
                self.wire_check_corner(target.below());
            }
        }
    }

    // ---- diodes -----------------------------------------------------------------

    /// `DiodeBlock.updateNeighborsInFront`.
    fn diode_update_in_front(&mut self, pos: BlockPos, state: BlockStateId) {
        let block = self.block_id(state);
        let direction = self.facing(state);
        let front = pos.relative(direction.opposite(), 1);
        self.neighbor_changed_at(front, block);
        self.update_neighbors_except(front, block, direction);
    }

    /// `ObserverBlock.updateNeighborsInFront` (the same shape as diodes').
    fn observer_update_in_front(&mut self, pos: BlockPos, state: BlockStateId) {
        self.diode_update_in_front(pos, state);
    }

    /// `DiodeBlock.getInputSignal` (with the comparator's analog inputs).
    fn diode_input(&self, kind: Kind, pos: BlockPos, state: BlockStateId) -> i32 {
        let direction = self.facing(state);
        let target = pos.relative(direction, 1);
        let mut input = self.signal(target, direction);
        if input < 15 {
            input = input.max(self.wire_power(self.block(target)));
        }
        if kind != Kind::Comparator {
            return input;
        }
        let target_state = self.block(target);
        if let Some(analog) = self.analog_output(target_state, target) {
            analog
        } else if input < 15 && self.is_conductor(target_state) {
            // Item frames are not simulated yet.
            let behind = target.relative(direction, 1);
            self.analog_output(self.block(behind), behind).unwrap_or(input)
        } else {
            input
        }
    }

    /// `DiodeBlock.getAlternateSignal`.
    fn diode_alternate(&self, kind: Kind, pos: BlockPos, state: BlockStateId) -> i32 {
        let direction = self.facing(state);
        let (cw, ccw) = (direction.clockwise(), direction.counter_clockwise());
        let only_diodes = kind == Kind::Repeater;
        self.control_input_signal(pos.relative(cw, 1), cw, only_diodes).max(self.control_input_signal(pos.relative(ccw, 1), ccw, only_diodes))
    }

    fn repeater_locked(&self, pos: BlockPos, state: BlockStateId) -> bool {
        self.diode_alternate(Kind::Repeater, pos, state) > 0
    }

    /// `DiodeBlock.shouldTurnOn` / `ComparatorBlock.shouldTurnOn`.
    fn diode_should_turn_on(&self, kind: Kind, pos: BlockPos, state: BlockStateId) -> bool {
        let input = self.diode_input(kind, pos, state);
        if kind != Kind::Comparator {
            return input > 0;
        }
        if input == 0 {
            return false;
        }
        let side = self.diode_alternate(kind, pos, state);
        input > side || input == side && self.prop(state, "mode") == Some("compare")
    }

    /// `DiodeBlock.shouldPrioritize`: the diode behind faces another way.
    fn diode_should_prioritize(&self, pos: BlockPos, state: BlockStateId) -> bool {
        let direction = self.facing(state).opposite();
        let behind = self.block(pos.relative(direction, 1));
        matches!(self.redstone_kind(behind), Some(Kind::Repeater | Kind::Comparator)) && self.facing(behind) != direction
    }

    /// `DiodeBlock.checkTickOnNeighbor` for repeaters.
    fn diode_check_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        if self.repeater_locked(pos, state) {
            return;
        }
        let block = self.block_id(state);
        let on = self.flag(state, "powered");
        if on != self.diode_should_turn_on(Kind::Repeater, pos, state) && !self.will_tick_this_tick(pos, block) {
            let priority = if self.diode_should_prioritize(pos, state) {
                priority::EXTREMELY_HIGH
            } else if on {
                priority::VERY_HIGH
            } else {
                priority::HIGH
            };
            let delay = self.int_prop(state, "delay") * 2;
            self.schedule_block_tick_priority(pos, block, delay, priority);
        }
    }

    /// `ComparatorBlock.calculateOutputSignal`.
    fn comparator_output_signal(&self, pos: BlockPos, state: BlockStateId) -> i32 {
        let input = self.diode_input(Kind::Comparator, pos, state);
        if input == 0 {
            return 0;
        }
        let side = self.diode_alternate(Kind::Comparator, pos, state);
        if side > input {
            0
        } else if self.prop(state, "mode") == Some("subtract") {
            input - side
        } else {
            input
        }
    }

    /// `ComparatorBlock.checkTickOnNeighbor`.
    fn comparator_check_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        let block = self.block_id(state);
        if self.will_tick_this_tick(pos, block) {
            return;
        }
        let output = self.comparator_output_signal(pos, state);
        let old = self.comparator_output_at(pos);
        if output != old || self.flag(state, "powered") != self.diode_should_turn_on(Kind::Comparator, pos, state) {
            let priority = if self.diode_should_prioritize(pos, state) { priority::HIGH } else { priority::NORMAL };
            self.schedule_block_tick_priority(pos, block, 2, priority);
        }
    }

    /// `ComparatorBlock.refreshOutputState`.
    fn comparator_refresh(&mut self, pos: BlockPos, state: BlockStateId) {
        let output = self.comparator_output_signal(pos, state);
        let old = self.comparator_output_at(pos);
        self.set_comparator_output(pos, output);
        if old != output || self.prop(state, "mode") == Some("compare") {
            let should = self.diode_should_turn_on(Kind::Comparator, pos, state);
            let on = self.flag(state, "powered");
            if on && !should {
                self.set_block(pos, self.with_bool(state, "powered", false), update::CLIENTS, update::LIMIT);
            } else if !on && should {
                self.set_block(pos, self.with_bool(state, "powered", true), update::CLIENTS, update::LIMIT);
            }
            self.diode_update_in_front(pos, state);
        }
    }

    // ---- torches -----------------------------------------------------------------

    /// `RedstoneTorchBlock.notifyNeighbors`.
    fn torch_notify(&mut self, pos: BlockPos, block: BlockId) {
        for direction in Direction::ALL {
            self.update_neighbors_at(pos.relative(direction, 1), block);
        }
    }

    /// `hasNeighborSignal` of a torch: the block it stands on or hangs from.
    fn torch_input(&self, kind: Kind, state: BlockStateId, pos: BlockPos) -> bool {
        if kind == Kind::WallTorch {
            let behind = self.facing(state).opposite();
            self.has_signal(pos.relative(behind, 1), behind)
        } else {
            self.has_signal(pos.below(), Direction::Down)
        }
    }

    /// `RedstoneTorchBlock.isToggledTooFrequently`.
    fn torch_toggled_too_often(&mut self, pos: BlockPos, add: bool) -> bool {
        if add {
            self.torch_toggles.push((pos, self.game_time));
        }
        self.torch_toggles.iter().filter(|(p, _)| *p == pos).count() >= 8
    }
}

/// The iteration order of vanilla's `HashSet<BlockPos>` holding a position
/// and its six neighbours (added in `Direction` order): by hash bucket of
/// a 16-bucket table, then insertion order.
fn java_hash_set_order(pos: BlockPos) -> Vec<BlockPos> {
    let mut all = vec![pos];
    all.extend(Direction::ALL.map(|d| pos.relative(d, 1)));
    let bucket = |p: &BlockPos| {
        // `Vec3i.hashCode`, then `HashMap.hash`.
        let h = (p.y.wrapping_add(p.z.wrapping_mul(31))).wrapping_mul(31).wrapping_add(p.x);
        ((h ^ ((h as u32) >> 16) as i32) & 15) as usize
    };
    all.sort_by_key(bucket);
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_set_order_is_by_bucket() {
        let order = java_hash_set_order(BlockPos::new(1, 81, 0));
        // Hashes: (y + z*31)*31 + x.
        let buckets: Vec<usize> = order
            .iter()
            .map(|p| {
                let h = (p.y + p.z * 31) * 31 + p.x;
                ((h ^ ((h as u32 >> 16) as i32)) & 15) as usize
            })
            .collect();
        assert!(buckets.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(order.len(), 7);
    }

    #[test]
    fn attached_directions() {
        assert_eq!(connected_direction(Some("floor"), Direction::North), Direction::Up);
        assert_eq!(connected_direction(Some("wall"), Direction::East), Direction::East);
    }
}
