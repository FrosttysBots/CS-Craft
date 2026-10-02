//! Local Overworld chat, styled like the CS HUD. The world is single-player.
use crate::{
    effects::{Effects, KillFeedEntry},
    player::{LocalPlayer, SimWorld},
    LoadedLevel,
};
use bevy::{
    input::{keyboard::KeyboardInput, ButtonState},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

#[derive(Clone)]
struct Line {
    text: String,
    color: Color,
    time: f32,
}
#[derive(Resource, Default)]
pub struct Chat {
    pub open: bool,
    team: bool,
    draft: String,
    lines: std::collections::VecDeque<Line>,
    scroll: usize,
    root: Option<Entity>,
    signature: String,
    welcomed: bool,
}
impl Chat {
    fn push(&mut self, text: String, color: Color, now: f32) {
        self.lines.push_back(Line {
            text,
            color,
            time: now,
        });
        while self.lines.len() > 64 {
            self.lines.pop_front();
        }
        self.scroll = 0;
    }
    fn system(&mut self, text: String, now: f32) {
        self.push(text, Color::srgb(0.88, 0.84, 0.66), now);
    }
    fn submit(&mut self, name: &str, alive: bool, now: f32) {
        let text = self.draft.trim().to_owned();
        if !text.is_empty() {
            self.push(
                format!(
                    "{}{}{} : {}",
                    if alive { "" } else { "*DEAD* " },
                    if self.team { "(TEAM) " } else { "" },
                    name,
                    text
                ),
                Color::srgb(0.65, 0.81, 1.),
                now,
            );
        }
        self.draft.clear();
        self.open = false;
    }
    fn append(&mut self, text: &str) {
        let remaining = 160usize.saturating_sub(self.draft.chars().count());
        self.draft
            .extend(text.chars().filter(|c| !c.is_control()).take(remaining));
    }
}
pub fn display_name(id: &str) -> String {
    id.trim_start_matches("minecraft:")
        .split('_')
        .map(|word| {
            let mut c = word.chars();
            match c.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), c.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn death_text(cause: &str, victim: &str) -> String {
    if let Some((mob, method)) = cause.strip_prefix("mob:").and_then(|s| s.split_once('|')) {
        format!("{}  [{method}]  {victim}", display_name(mob))
    } else {
        format!("[{}]  {victim}", if cause=="planted_c4" {"bomb"}else{cause.trim_start_matches("weapon_")})
    }
}
#[allow(clippy::too_many_arguments)]
pub fn input(
    mut chat: ResMut<Chat>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut scroll: ResMut<bevy::input::mouse::AccumulatedMouseScroll>,
    mut events: MessageReader<KeyboardInput>,
    mut local: ResMut<LocalPlayer>,
    world: Res<SimWorld>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    front: Res<crate::menu::Menu>,
    buy: Res<crate::buymenu::BuyMenu>,
    building: Res<crate::overworld_client::Building>,
    level: Res<LoadedLevel>,
    enabled: Res<crate::player::InputEnabled>,
    time: Res<Time>,
) {
    let events: Vec<_> = events.read().cloned().collect();
    if !enabled.0 || level.0.overworld.is_none() {
        return;
    }
    if front.open() || buy.open || building.inventory_open {
        chat.open = false;
        return;
    }
    let was_open = chat.open;
    if !was_open {
        if keys.just_pressed(KeyCode::KeyY) || keys.just_pressed(KeyCode::KeyU) {
            chat.open = true;
            chat.team = keys.just_pressed(KeyCode::KeyU);
            chat.draft.clear();
            chat.scroll = 0;
        } else {
            return;
        }
    } else {
        for event in events
            .into_iter()
            .filter(|e| e.state == ButtonState::Pressed)
        {
            match event.key_code {
                KeyCode::Escape => {
                    chat.draft.clear();
                    chat.open = false;
                    break;
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let p = world.0.player(local.id);
                    chat.submit(
                        p.map_or("You", |p| p.name.as_str()),
                        p.is_some_and(|p| p.alive),
                        time.elapsed_secs(),
                    );
                    break;
                }
                KeyCode::Backspace => {
                    chat.draft.pop();
                }
                KeyCode::PageUp => {
                    chat.scroll = (chat.scroll + 5).min(chat.lines.len().saturating_sub(8));
                }
                KeyCode::PageDown => {
                    chat.scroll = chat.scroll.saturating_sub(5);
                }
                _ => {
                    if let Some(text) = event.text {
                        chat.append(&text);
                    }
                }
            }
        }
        if scroll.delta.y > 0. {
            chat.scroll = (chat.scroll + 3).min(chat.lines.len().saturating_sub(8));
        }
        if scroll.delta.y < 0. {
            chat.scroll = chat.scroll.saturating_sub(3);
        }
    }
    // Consume typing before every gameplay input system, including the closing frame.
    keys.reset_all();
    mouse.reset_all();
    scroll.delta = Vec2::ZERO;
    local.latched = 0;
    local.pending_slot = 0;
    local.pending_drop = false;
    cursor.grab_mode = if chat.open {
        CursorGrabMode::None
    } else {
        CursorGrabMode::Locked
    };
    cursor.visible = chat.open;
}
#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut commands: Commands,
    mut chat: ResMut<Chat>,
    mut effects: ResMut<Effects>,
    level: Res<LoadedLevel>,
    world: Res<SimWorld>,
    local: Res<LocalPlayer>,
    front: Res<crate::menu::Menu>,
    ui: Res<crate::ui_assets::UiAssets>,
    time: Res<Time>,
    enabled: Res<crate::player::InputEnabled>,
    mut staged: Local<bool>,
) {
    let Some(runtime) = &level.0.overworld else {
        return;
    };
    let now = time.elapsed_secs();
    if !chat.welcomed {
        chat.system(
            "Welcome to Overworld. Y: chat | U: team chat | Enter: send | Esc: cancel".into(),
            now,
        );
        chat.welcomed = true;
    }
    {
        let mut r = runtime.lock().unwrap();
        for text in std::mem::take(&mut r.notices) {
            chat.system(text, now);
        }
        let player = world.0.player(local.id).map_or("You", |p| p.name.as_str());
        for mob in std::mem::take(&mut r.mob_kills) {
            let text = format!("{player}  [kill]  {}", display_name(&mob));
            effects.kill_feed.push(KillFeedEntry {
                text: text.clone(),
                local_involved: true,
                time: now,
            });
            chat.system(text, now);
        }
    }
    for text in std::mem::take(&mut effects.chat_events) {
        chat.system(text, now);
    }
    // Deterministic presentation fixture, restricted to screenshot runs.
    if !enabled.0 && !*staged && std::env::var_os("CSR_TEST_CHAT").is_some() && now > 1. {
        *staged = true;
        chat.draft = "Overworld ready!".into();
        chat.submit("You", true, now);
        for text in ["You  [kill]  Zombie", "Skeleton  [arrow]  You"] {
            chat.system(text.into(), now);
            effects.kill_feed.push(KillFeedEntry {
                text: text.into(),
                local_involved: true,
                time: now,
            });
        }
        chat.system("+$100 — zombie".into(), now);
        chat.open = true;
        chat.draft = "Anyone need supplies?".into();
    }
    if effects.kill_feed.len() > 6 {
        let excess = effects.kill_feed.len() - 6;
        effects.kill_feed.drain(..excess);
    }
    let mut rows = Vec::new();
    if !front.open() && std::env::var_os("CSR_NO_HUD").is_none() {
        let mut lines: Vec<_> = chat
            .lines
            .iter()
            .rev()
            .skip(if chat.open { chat.scroll } else { 0 })
            .filter(|l| chat.open || now - l.time < 12.)
            .take(8)
            .cloned()
            .collect();
        lines.reverse();
        rows = lines;
        if chat.open {
            rows.push(Line {
                text: format!(
                    "{} : {}_",
                    if chat.team { "Say to team" } else { "Say" },
                    chat.draft
                ),
                color: Color::WHITE,
                time: now,
            });
        }
    }
    let signature = format!(
        "{}|{}|{:?}",
        chat.open,
        chat.scroll,
        rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>()
    );
    if signature == chat.signature {
        return;
    }
    chat.signature = signature;
    if let Some(root) = chat.root.take() {
        commands.entity(root).despawn();
    }
    if rows.is_empty() {
        return;
    }
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(20),
                bottom: px(185),
                width: percent(42),
                max_width: px(640),
                padding: UiRect::all(px(10)),
                row_gap: px(3),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(
                0.02,
                0.025,
                0.03,
                if chat.open { 0.78 } else { 0.38 },
            )),
            GlobalZIndex(25),
        ))
        .id();
    for line in rows {
        commands.entity(root).with_children(|p| {
            p.spawn((
                Text::new(line.text),
                TextFont {
                    font: ui.regular.clone().into(),
                    font_size: FontSize::Px(17.),
                    ..default()
                },
                TextColor(line.color),
                Node {
                    max_width: percent(100),
                    ..default()
                },
            ));
        });
    }
    chat.root = Some(root);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_keeps_unicode_limits_and_discards_empty_messages() {
        let mut c = Chat::default();
        c.append("héllo\n");
        assert_eq!(c.draft, "héllo");
        c.append(&"界".repeat(200));
        assert_eq!(c.draft.chars().count(), 160);
        c.team = true;
        c.submit("You", false, 1.);
        assert!(c.lines[0].text.starts_with("*DEAD* (TEAM) You : héllo"));
        c.draft = "   ".into();
        c.submit("You", true, 2.);
        assert_eq!(c.lines.len(), 1);
        for _ in 0..100 {
            c.system("test".into(), 2.);
        }
        assert_eq!(c.lines.len(), 64);
    }
    #[test]
    fn feed_preserves_attacker_and_attack_kind() {
        assert_eq!(
            death_text("mob:minecraft:wither_skeleton|arrow", "You"),
            "Wither Skeleton  [arrow]  You"
        );
        assert_eq!(death_text("survival", "You"), "[survival]  You");
    }
}


#[cfg(test)]
mod integration {
    use super::*;
    use bevy::input::keyboard::Key;
    #[test]
    #[ignore="requires installed CS/MC assets and an isolated CSR_WORLD_DIR"]
    fn overworld_chat_consumes_input_and_routes_mob_deaths_once() {
        assert!(std::env::var_os("CSR_WORLD_DIR").is_some());
        let level=level::Level::load_seed("overworld",None,Some(853109)).unwrap();
        let sounds=crate::audio::SoundLibrary::load(&level.vfs);
        let mut w=sim::World::with_catalog(level.collision.clone(),level.catalog.clone());
        w.set_mode(sim::GameMode::Special,Default::default());
        let id=w.add_player("You",sim::Team::CounterTerrorist);
        {let p=w.player_mut(id).unwrap();p.alive=true;p.health=100;}
        let mut a=App::new();
        a.insert_resource(LoadedLevel(level)).insert_resource(SimWorld(w)).insert_resource(sounds)
            .insert_resource(LocalPlayer::new(id,Vec3::ZERO,0.)).init_resource::<Chat>()
            .init_resource::<Effects>().init_resource::<crate::effects::PendingEvents>()
            .init_resource::<crate::tracers::Tracers>().init_resource::<Assets<AudioSource>>()
            .init_resource::<crate::menu::Menu>().init_resource::<crate::buymenu::BuyMenu>()
            .init_resource::<crate::overworld_client::Building>().init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>().init_resource::<bevy::input::mouse::AccumulatedMouseScroll>()
            .init_resource::<Time>().insert_resource(crate::player::InputEnabled(true))
            .insert_resource(crate::ui_assets::UiAssets{regular:default(),bold:default(),images:default()})
            .add_message::<KeyboardInput>().add_systems(Update,(input,crate::effects::process,draw).chain());
        let window=a.world_mut().spawn((PrimaryWindow,CursorOptions{grab_mode:CursorGrabMode::Locked,visible:false,..default()})).id();
        a.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyY);a.update();
        assert!(a.world().resource::<Chat>().open);
        let send=|a:&mut App,key_code:KeyCode,text:&str| {
            a.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(key_code);
            a.world_mut().resource_mut::<Messages<KeyboardInput>>().write(KeyboardInput{key_code,logical_key:Key::Character(text.into()),state:ButtonState::Pressed,text:Some(text.into()),repeat:false,window});
            a.update();
        };
        a.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        a.world_mut().resource_mut::<LocalPlayer>().latched=movement::buttons::ATTACK;
        send(&mut a,KeyCode::KeyB,"buy inventory options");
        assert!(!a.world().resource::<ButtonInput<KeyCode>>().pressed(KeyCode::KeyB));
        assert!(!a.world().resource::<ButtonInput<MouseButton>>().pressed(MouseButton::Left));
        assert_eq!(a.world().resource::<LocalPlayer>().latched,0);
        send(&mut a,KeyCode::Enter,"");
        assert!(!a.world().resource::<Chat>().open);
        assert!(a.world().resource::<Chat>().lines.iter().any(|l|l.text=="You : buy inventory options"));
        assert_eq!(a.world().get::<CursorOptions>(window).unwrap().grab_mode,CursorGrabMode::Locked);
        a.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyU);a.update();
        send(&mut a,KeyCode::Escape,"");assert!(!a.world().resource::<Chat>().open);
        assert!(!a.world().resource::<ButtonInput<KeyCode>>().just_pressed(KeyCode::Escape));
        {
            let level=a.world().resource::<LoadedLevel>();let mut r=level.0.overworld.as_ref().unwrap().lock().unwrap();
            r.reward_mob("minecraft:cow");r.reward_mob("minecraft:zombie");
        }
        a.update();assert_eq!(a.world().resource::<Effects>().kill_feed.len(),2);
        a.update();assert_eq!(a.world().resource::<Effects>().kill_feed.len(),2,"no duplicate rewards/feed entries");
        // Feed is driven by an actual fatal sim hit, not by every mob damage event.
        let hit=overworld::MobPlayerHit::native(overworld::mc_entities::world::PlayerHit{player_id:1,damage:3.,kind:overworld::mc_entities::world::PlayerHitKind::Arrow{velocity:overworld::glam_mc::DVec3::ZERO,effect:None},source:Some(12)},Some("minecraft:skeleton"));
        {let mut w=a.world_mut().resource_mut::<SimWorld>();w.0.events.clear();w.0.damage_environment(id,1,&hit.cause());}
        let events=a.world().resource::<SimWorld>().0.events.clone();a.world_mut().resource_mut::<crate::effects::PendingEvents>().0.extend(events);a.update();
        assert_eq!(a.world().resource::<Effects>().kill_feed.len(),2);
        {let mut w=a.world_mut().resource_mut::<SimWorld>();w.0.events.clear();w.0.damage_environment(id,200,&hit.cause());w.0.damage_environment(id,200,&hit.cause());}
        let events=a.world().resource::<SimWorld>().0.events.clone();a.world_mut().resource_mut::<crate::effects::PendingEvents>().0.extend(events);a.update();
        let feed=&a.world().resource::<Effects>().kill_feed;
        assert_eq!(feed.len(),3);assert_eq!(feed[2].text,"Skeleton  [arrow]  You");
        assert!(a.world().resource::<Chat>().lines.iter().any(|l|l.text=="Skeleton  [arrow]  You"));
        println!("PASS: chat input isolation, submit/cancel, player mob kills, named fatal mob attacks, nonfatal and duplicate suppression.");
    }
}
