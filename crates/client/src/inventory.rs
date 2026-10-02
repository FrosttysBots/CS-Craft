//! Local base-item collection and persisted practice loadouts.
use bevy::prelude::*;
use std::sync::Arc;
use weapons::{Catalog, Side, WeaponClass, WeaponDefinition};

#[derive(Resource, Default)]
pub struct Collection(pub Arc<Catalog>);
#[derive(Resource)]
pub struct Inventory {
    pub selected: String,
    pub filter: usize,
    pub yaw: f32,
    pub loadouts: [String; 4],
}
impl Default for Inventory {
    fn default() -> Self {
        Self {
            selected: "weapon_ak47".into(),
            filter: 0,
            yaw: 0.0,
            loadouts: [
                "weapon_ak47",
                "weapon_glock",
                "weapon_m4a1_silencer",
                "weapon_usp_silencer",
            ]
            .map(str::to_string),
        }
    }
}
pub const FILTERS: [&str; 7] = [
    "ALL", "PISTOLS", "RIFLES", "SMGs", "SNIPERS", "HEAVY", "KNIVES",
];
pub fn visible(def: &WeaponDefinition, filter: usize) -> bool {
    use WeaponClass::*;
    if def.def_index >= 500 {return false;}
    match filter {
        0 => matches!(
            def.class,
            Knife | Pistol | Rifle | Smg | Sniper | Shotgun | MachineGun
        ),
        1 => def.class == Pistol,
        2 => def.class == Rifle,
        3 => def.class == Smg,
        4 => def.class == Sniper,
        5 => matches!(def.class, Shotgun | MachineGun),
        6 => def.class == Knife,
        _ => false,
    }
}
pub fn allowed(def: &WeaponDefinition, ct: bool) -> bool {
    let slot = sim::weapon::slot(def);
    (slot == 1 || slot == 2)
        && (def.side == Side::Both
            || def.side
                == if ct {
                    Side::CounterTerrorist
                } else {
                    Side::Terrorist
                })
}
impl Inventory {
    pub fn load() -> Self {
        let mut out = Self::default();
        if let Ok(text) = std::fs::read_to_string("config/inventory.cfg") {
            out.decode(&text);
        }
        out
    }
    fn decode(&mut self, text: &str) {
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let Ok(i) = key.parse::<usize>() else {
                continue;
            };
            if i < 4
                && value.starts_with("weapon_")
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                self.loadouts[i] = value.to_string();
            }
        }
    }
    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all("config")?;
        std::fs::write(
            "config/inventory.tmp",
            self.loadouts
                .iter()
                .enumerate()
                .map(|(i, s)| format!("{i}={s}\n"))
                .collect::<String>(),
        )?;
        std::fs::rename("config/inventory.tmp", "config/inventory.cfg")
    }
    pub fn equip(&mut self, catalog: &Catalog, ct: bool) -> bool {
        let Some(def) = catalog.get(&self.selected).filter(|d| allowed(d, ct)) else {
            return false;
        };
        let index = usize::from(ct) * 2 + usize::from(sim::weapon::slot(def) == 2);
        self.loadouts[index] = def.class_name.clone();
        true
    }
}
pub fn apply(world: &mut sim::World, id: sim::PlayerId, inventory: &Inventory) {
    // Collection choices grant weapons only in the respawning practice mode.
    if world.mode != sim::GameMode::Deathmatch {
        return;
    }
    let Some(p) = world.player(id) else { return };
    let ct = p.team == sim::Team::CounterTerrorist;
    let defs: Vec<_> = (0..2)
        .filter_map(|slot| {
            world
                .catalog
                .get(&inventory.loadouts[usize::from(ct) * 2 + slot])
                .filter(|d| allowed(d, ct) && sim::weapon::slot(d) == slot as u8 + 1)
                .cloned()
        })
        .collect();
    let Some(p) = world.player_mut(id) else {
        return;
    };
    for def in defs {
        p.weapons
            .retain(|w| sim::weapon::slot(&w.def) != sim::weapon::slot(&def));
        p.weapons.push(sim::weapon::WeaponState::new(Arc::new(def)));
    }
    p.active = p.best_weapon();
    p.last_active = p.active;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog() -> Arc<Catalog> {
        Arc::new(
            Catalog::from_items_game(
                r#""items_game" { "items" {
            "7" { "name" "weapon_ak47" "baseitem" "1" "used_by_classes" { "terrorists" "1" } }
            "4" { "name" "weapon_glock" "baseitem" "1" "used_by_classes" { "terrorists" "1" } }
            "9" { "name" "weapon_awp" "baseitem" "1" }
            "42" { "name" "weapon_knife" "baseitem" "1" }
        } }"#,
            )
            .unwrap(),
        )
    }
    #[test]
    fn practice_loadout_cannot_grant_survival_weapons() {
        let mut world=sim::World::with_catalog(Arc::new(collision::CollisionWorld::new()),catalog());
        world.set_mode(sim::GameMode::Special,Default::default());
        let id=world.add_player("survivor",sim::Team::Terrorist);
        let before:Vec<_>=world.player(id).unwrap().weapons.iter().map(|w|w.def.class_name.clone()).collect();
        apply(&mut world,id,&Inventory::default());
        assert_eq!(before,world.player(id).unwrap().weapons.iter().map(|w|w.def.class_name.clone()).collect::<Vec<_>>());
    }
    #[test]
    fn team_and_slot_validation_and_deathmatch_application() {
        let catalog = catalog();
        let mut inv = Inventory::default();
        assert!(!inv.equip(&catalog, true));
        inv.selected = "weapon_awp".into();
        assert!(inv.equip(&catalog, true));
        assert_eq!(inv.loadouts[2], "weapon_awp");
        inv.selected = "weapon_knife".into();
        assert!(!inv.equip(&catalog, true));
        let mut world =
            sim::World::with_catalog(Arc::new(collision::CollisionWorld::new()), catalog);
        let id = world.add_player("Test", sim::Team::CounterTerrorist);
        apply(&mut world, id, &inv);
        assert_eq!(
            world
                .player(id)
                .unwrap()
                .active_weapon()
                .unwrap()
                .def
                .class_name,
            "weapon_awp"
        );
        // Edited configuration cannot bypass team restrictions or put a pistol in a primary slot.
        inv.loadouts[2] = "weapon_glock".into();
        apply(&mut world, id, &inv);
        assert!(
            world
                .player(id)
                .unwrap()
                .weapons
                .iter()
                .all(|w| w.def.class_name != "weapon_glock")
        );
        world.set_mode(sim::GameMode::Competitive, Default::default());
        let before: Vec<_> = world
            .player(id)
            .unwrap()
            .weapons
            .iter()
            .map(|w| w.def.class_name.clone())
            .collect();
        inv.loadouts[2] = "weapon_awp".into();
        apply(&mut world, id, &inv);
        assert_eq!(
            before,
            world
                .player(id)
                .unwrap()
                .weapons
                .iter()
                .map(|w| w.def.class_name.clone())
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn invalid_config_cannot_supply_paths_or_extra_slots() {
        let mut inv = Inventory::default();
        let before = inv.loadouts.clone();
        inv.decode("0=../../other\n4=weapon_awp\n2=weapon_awp\n1=weapon_glock\n");
        assert_eq!(inv.loadouts[0], before[0]);
        assert_eq!(inv.loadouts[2], "weapon_awp");
    }
}
