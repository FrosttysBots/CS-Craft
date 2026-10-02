//! Checks weapon behaviour and animation/sound availability against the installed game.
use std::sync::Arc;
use movement::buttons;
use sim::{GameEvent, Team, TickInput, World};
fn main() {
    let (_, fs) = level::mount_game(None).unwrap();
    let catalog = Arc::new(weapons::Catalog::from_items_game(&fs.read_string("scripts/items/items_game.txt").unwrap()).unwrap());
    let sounds = client::audio::SoundLibrary::load(&fs);
    for name in ["Weapon_Knife.Slash", "Weapon_Knife.Hit", "Weapon_Knife.HitWall", "Weapon_Knife.Stab"] {
        assert!(sounds.has(name), "missing {name}");
    }
    for name in ["weapon_knife", "weapon_knife_t", "weapon_ak47", "weapon_awp", "weapon_ssg08", "weapon_g3sg1", "weapon_scar20", "weapon_aug", "weapon_sg556"] {
        let def = catalog.get(name).unwrap();
        let model = mdl::Model::load_header(&fs.read(&def.view_model).unwrap()).unwrap();
        let mut activities: Vec<_> = model.sequences.iter().map(|s| s.activity.clone()).collect();
        for include in &model.include_models {
            let model = mdl::Model::load_header(&fs.read(include).unwrap()).unwrap();
            activities.extend(model.sequences.iter().map(|s| s.activity.clone()));
        }
        activities.sort(); activities.dedup();
        assert!(activities.iter().any(|a| a.eq_ignore_ascii_case("ACT_VM_DRAW")), "{name} draw sequence");
        if def.class == weapons::WeaponClass::Knife {
            for activity in ["ACT_VM_MISSCENTER", "ACT_VM_MISSCENTER2"] {
                assert!(activities.iter().any(|a| a.eq_ignore_ascii_case(activity)), "{name}: {activity} in {activities:?}");
            }
        }
        let mut world = World::with_catalog(Arc::new(collision::CollisionWorld::new()), catalog.clone());
        let id = world.add_player("check", Team::CounterTerrorist);
        let p = world.player_mut(id).unwrap();
        p.weapons = vec![sim::weapon::WeaponState::new(Arc::new(def.clone()))]; p.active = 0;
        let levels = p.active_weapon().unwrap().zoom_levels();
        if levels > 0 {
            assert!(!def.zoom_in_sound.is_empty() && sounds.has(&def.zoom_in_sound), "{name} zoom-in sound");
            assert!(!def.zoom_out_sound.is_empty() && sounds.has(&def.zoom_out_sound), "{name} zoom-out sound");
            for expected in (1..=levels).chain(std::iter::once(0)) {
                world.step(&[(id, TickInput::default())]);
                world.step(&[(id, TickInput { buttons: buttons::ATTACK2, ..Default::default() })]);
                assert!(world.events.iter().any(|e| matches!(e, GameEvent::Scope { level, .. } if *level == expected)));
                for _ in 0..24 { world.step(&[(id, TickInput::default())]); }
                assert_eq!(world.player(id).unwrap().active_weapon().unwrap().zoom_level, expected);
            }
        } else if def.class == weapons::WeaponClass::Knife {
            world.step(&[(id, TickInput { buttons: buttons::ATTACK2, ..Default::default() })]);
            assert!(world.events.iter().any(|e| matches!(e, GameEvent::Melee { secondary: true, .. })));
            assert!(!world.events.iter().any(|e| matches!(e, GameEvent::Fire { .. } | GameEvent::Tracer { .. })));
        }
        println!("{name}: original draw animation OK, zoom levels {levels}, controls OK");
    }
    println!("weapon checks: failures: 0");
}
