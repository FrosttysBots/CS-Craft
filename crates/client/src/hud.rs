//! HUD: crosshair, health/armour, money, ammo and inventory, the round
//! timer and score, bomb status, plant/defuse progress, kill feed, round
//! banners, spectating, hit marker and a debug line.

use crate::effects::Effects;
use crate::localize::Localization;
use crate::player::{LocalPlayer, SimWorld};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use sim::{Phase, Team};

/// Which piece of HUD text a node shows.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum HudText {
    Speed,
    Debug,
    Health,
    Armor,
    Money,
    Ammo,
    Inventory,
    Feed,
    Center,
    Hint,
    ScoreCt,
    Timer,
    ScoreT,
    Phase,
    Progress,
}

#[derive(Component)]
pub struct HitMarker;

#[derive(Component)]
pub struct Crosshair(pub u8);

#[derive(Component)]
pub struct ProgressBar;

#[derive(Component)]
pub struct ProgressFill;

const HUD_YELLOW: Color = Color::srgb(0.88, 0.89, 0.87);
const MONEY_GREEN: Color = Color::srgb(0.88, 0.89, 0.87);
const CT_BLUE: Color = Color::srgb(0.45, 0.65, 1.0);
const T_GOLD: Color = Color::srgb(0.95, 0.75, 0.3);
const BOMB_RED: Color = Color::srgb(1.0, 0.25, 0.2);

fn text(commands: &mut Commands, kind: HudText, size: f32, color: Color, node: Node) -> Entity {
    commands
        .spawn((
            Text::new(""),
            TextFont {
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
            node,
            kind,
        ))
        .id()
}

fn centered_row(top: Val, bottom: Val) -> Node {
    Node {
        position_type: PositionType::Absolute,
        top,
        bottom,
        width: percent(100),
        justify_content: JustifyContent::Center,
        ..default()
    }
}

pub fn setup(mut commands: Commands) {
    // Crosshair: four green bars with a gap, CS:GO "classic static" style,
    // plus a hit marker (hidden until a hit).
    let color = Color::srgb(0.2, 1.0, 0.2);
    let (len, thick, gap) = (8.0, 2.0, 4.0);
    let bars = [
        (-(gap + len), -thick / 2.0, len, thick),
        (gap, -thick / 2.0, len, thick),
        (-thick / 2.0, -(gap + len), thick, len),
        (-thick / 2.0, gap, thick, len),
        (-thick / 2.0, -thick / 2.0, thick, thick),
    ];
    commands
        .spawn(Node {
            width: percent(100),
            height: percent(100),
            position_type: PositionType::Absolute,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|root| {
            root.spawn(Node {
                width: px(0.0),
                height: px(0.0),
                ..default()
            })
            .with_children(|c| {
                for (i, (x, y, w, h)) in bars.into_iter().enumerate() {
                    c.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(x),
                            top: px(y),
                            width: px(w),
                            height: px(h),
                            ..default()
                        },
                        BackgroundColor(color),
                        Crosshair(i as u8),
                    ));
                }
                for (x, y) in [(-12.0, -12.0), (7.0, -12.0), (-12.0, 7.0), (7.0, 7.0)] {
                    c.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(x),
                            top: px(y),
                            width: px(5.0),
                            height: px(5.0),
                            ..default()
                        },
                        BackgroundColor(Color::WHITE),
                        Visibility::Hidden,
                        HitMarker,
                    ));
                }
            });
        });

    let speed = text(&mut commands, HudText::Speed, 20.0, Color::srgba(1.0, 1.0, 1.0, 0.8), centered_row(Val::Auto, px(90.0)));
    commands.entity(speed).insert(TextLayout::justify(Justify::Center));
    text(
        &mut commands,
        HudText::Debug,
        13.0,
        Color::srgba(1.0, 1.0, 1.0, 0.75),
        Node {
            position_type: PositionType::Absolute,
            top: px(6.0),
            left: px(8.0),
            ..default()
        },
    );
    text(
        &mut commands,
        HudText::Health,
        34.0,
        HUD_YELLOW,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(16.0),
            left: px(66.0),
            ..default()
        },
    );
    text(&mut commands, HudText::Armor, 34.0, HUD_YELLOW, Node { position_type: PositionType::Absolute, bottom: px(16.0), left: px(217.0), ..default() });
    text(
        &mut commands,
        HudText::Money,
        28.0,
        MONEY_GREEN,
        Node {
            position_type: PositionType::Absolute,
            top: px(286.0),
            left: px(24.0),
            ..default()
        },
    );
    let ammo = text(
        &mut commands,
        HudText::Ammo,
        34.0,
        HUD_YELLOW,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(16.0),
            right: px(24.0),
            ..default()
        },
    );
    commands.entity(ammo).insert(TextLayout::justify(Justify::Right));
    let inv = text(
        &mut commands,
        HudText::Inventory,
        17.0,
        Color::srgba(1.0, 1.0, 1.0, 0.8),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(110.0),
            right: px(24.0),
            ..default()
        },
    );
    commands.entity(inv).insert(TextLayout::justify(Justify::Right));
    let feed = text(
        &mut commands,
        HudText::Feed,
        16.0,
        Color::WHITE,
        Node {
            position_type: PositionType::Absolute,
            top: px(60.0),
            right: px(16.0),
            ..default()
        },
    );
    commands.entity(feed).insert(TextLayout::justify(Justify::Right));
    let center = text(&mut commands, HudText::Center, 32.0, Color::WHITE, centered_row(percent(24), Val::Auto));
    commands.entity(center).insert(TextLayout::justify(Justify::Center));
    let hint = text(&mut commands, HudText::Hint, 20.0, Color::WHITE, centered_row(Val::Auto, px(150.0)));
    commands.entity(hint).insert(TextLayout::justify(Justify::Center));

    // Top centre: CT score | timer | T score, with the phase under it.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(8.0),
                width: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|top| {
            top.spawn((
                Node {
                    column_gap: px(18.0),
                    padding: UiRect::axes(px(14.0), px(4.0)),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            ))
            .with_children(|row| {
                for (kind, color, size) in [(HudText::ScoreCt, CT_BLUE, 26.0), (HudText::Timer, Color::WHITE, 26.0), (HudText::ScoreT, T_GOLD, 26.0)] {
                    row.spawn((
                        Text::new(""),
                        TextFont {
                            font_size: FontSize::Px(size),
                            ..default()
                        },
                        TextColor(color),
                        kind,
                    ));
                }
            });
            top.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
                HudText::Phase,
            ));
        });

    // Plant / defuse progress bar.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: percent(62),
                width: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(4.0),
                ..default()
            },
            Visibility::Hidden,
            ProgressBar,
        ))
        .with_children(|bar| {
            bar.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                HudText::Progress,
            ));
            bar.spawn((
                Node {
                    width: px(300.0),
                    height: px(10.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            ))
            .with_children(|track| {
                track.spawn((
                    Node {
                        width: percent(0),
                        height: percent(100),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.95, 0.8, 0.3)),
                    ProgressFill,
                ));
            });
        });
}

fn clock(seconds: f32) -> String {
    let s = seconds.ceil().max(0.0) as i32;
    format!("{}:{:02}", s / 60, s % 60)
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn update(
    local: Res<LocalPlayer>,
    world: Res<SimWorld>,
    effects: Res<Effects>,
    settings: Res<crate::settings::Settings>,
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    level_name: Res<crate::LevelInfo>,
    _loc: Res<Localization>,
    keys: Res<ButtonInput<KeyCode>>,
    mut debug: Local<bool>,
    mut texts: Query<(&mut Text, &mut TextColor, &HudText)>,
    mut markers: Query<&mut Visibility, (With<HitMarker>, Without<ProgressBar>)>,
    mut bar: Query<&mut Visibility, (With<ProgressBar>, Without<HitMarker>)>,
    mut fill: Query<&mut Node, With<ProgressFill>>,
) {
    if keys.just_pressed(KeyCode::F3) {
        *debug = !*debug;
    }
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let w = &world.0;
    let Some(me) = w.player(local.id) else { return };
    let now = time.elapsed_secs();
    let sim_now = w.time();
    // The player whose HUD we show (a spectated teammate's while dead).
    let p = local.spectating.and_then(|s| w.player(s)).unwrap_or(me);
    let competitive = w.competitive();

    // Plant / defuse progress for the viewed player.
    let progress: Option<(String, f32)> = if let Some(start) = p.plant_start {
        Some(("Planting the bomb".into(), (sim_now - start) / w.rules.plant_time))
    } else {
        w.bomb.and_then(|b| b.defuser).filter(|(d, _)| *d == p.id).map(|(_, done)| {
            let total = w.rules.defuse_duration(p.defuse_kit);
            (if p.defuse_kit { "Defusing the bomb (kit)".into() } else { "Defusing the bomb".into() }, 1.0 - (done - sim_now) / total)
        })
    };
    for mut v in &mut bar {
        *v = if progress.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    }
    if let Some((_, k)) = &progress {
        for mut n in &mut fill {
            n.width = percent((k.clamp(0.0, 1.0) * 100.0) as i32);
        }
    }

    let round = &w.round;

    for (mut t, mut color, kind) in &mut texts {
        let value: String = match kind {
            HudText::Speed | HudText::Debug if !*debug => String::new(),
            HudText::Speed => format!("{:.0}", p.movement.velocity.truncate().length()),
            HudText::Debug => format!(
                "{} | {:.0} fps\npos {:.0} {:.0} {:.0}\nK {} D {}{} | F3 hide",
                level_name.name,
                fps,
                local.curr_eye.x,
                local.curr_eye.y,
                local.curr_eye.z,
                me.kills,
                me.deaths,
                if local.noclip { " | NOCLIP" } else { "" },
            ),
            HudText::Health => if p.alive { p.health.to_string() } else { String::new() },
            HudText::Armor => if p.alive { p.armor.to_string() } else { String::new() },
            HudText::Money => if competitive || w.mode == sim::GameMode::Special { format!("${}", p.money) } else { String::new() },
            HudText::Ammo => match p.active_weapon() {
                Some(wp) if p.alive && wp.uses_ammo() => format!("{} / {}", wp.clip, wp.reserve),
                _ => String::new(),
            },
            HudText::Inventory => String::new(),
            HudText::Feed => effects
                .kill_feed
                .iter()
                .map(|k| if k.local_involved { format!("> {}", k.text) } else { k.text.clone() })
                .collect::<Vec<_>>()
                .join("\n"),
            HudText::Center => {
                let banner = effects.banner.as_ref().filter(|b| now - b.2 < 7.0);
                if let Some((text, c, _)) = banner {
                    color.0 = *c;
                    text.clone()
                } else if !me.alive && !local.noclip && !competitive {
                    color.0 = Color::srgb(1.0, 0.35, 0.3);
                    format!("YOU DIED - respawning in {:.1}s", (me.respawn_at - sim_now).max(0.0))
                } else {
                    String::new()
                }
            }
            HudText::Hint => {
                if !me.alive && !local.noclip {
                    match local.spectating.and_then(|s| w.player(s)) {
                        Some(s) => format!("Spectating {}   (LMB: next player)", s.name),
                        None if competitive => "Waiting for the next round".into(),
                        None => String::new(),
                    }
                } else if competitive && round.phase == Phase::Live && me.alive {
                    let (mn, mx) = me.bounds();
                    let in_site = w.objectives.site_at(mn, mx);
                    if me.has_bomb() && in_site.is_some() && me.plant_start.is_none() {
                        "In the bomb site: hold LMB with the C4 (5) to plant".into()
                    } else if me.team == Team::CounterTerrorist
                        && w.bomb.is_some_and(|b| b.defuser.is_none() && (b.pos - me.movement.origin).truncate().length() < sim::DEFUSE_RANGE)
                    {
                        "Look at the bomb and hold E to defuse".into()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            }
            HudText::ScoreCt => {
                if competitive { round.score(Team::CounterTerrorist).to_string() } else { String::new() }
            }
            HudText::ScoreT => {
                if competitive { round.score(Team::Terrorist).to_string() } else { String::new() }
            }
            HudText::Timer => {
                if !competitive {
                    String::new()
                } else if w.bomb.is_some() && round.phase == Phase::Live {
                    // The game hides the bomb timer; show a pulsing icon.
                    color.0 = if (now * 2.0).fract() < 0.5 { BOMB_RED } else { Color::srgb(0.6, 0.1, 0.1) };
                    "C4".into()
                } else {
                    color.0 = if round.phase == Phase::Live && round.time_left(&w.rules, sim_now) <= 10.0 { BOMB_RED } else { Color::WHITE };
                    clock(round.time_left(&w.rules, sim_now))
                }
            }
            HudText::Phase => {
                if !competitive {
                    String::new()
                } else {
                    let r = round.round;
                    match round.phase {
                        Phase::Freeze => format!("Round {r} - freeze time"),
                        Phase::Live if w.bomb.is_some() => format!("Round {r} - BOMB PLANTED"),
                        Phase::Live => format!("Round {r}{}", if me.has_bomb() { " - you have the bomb" } else { "" }),
                        Phase::Over => format!("Round {r} over"),
                        Phase::MatchOver => "Match over - new match soon".into(),
                    }
                }
            }
            HudText::Progress => progress.as_ref().map(|p| p.0.clone()).unwrap_or_default(),
        };
        if t.0 != value {
            t.0 = value;
        }
    }

    let show = settings.hitmarker && effects.last_hit > 0.0 && now - effects.last_hit < 0.18;
    for mut v in &mut markers {
        *v = if show { Visibility::Inherited } else { Visibility::Hidden };
    }
}

/// Hide gameplay UI during transparent menus without disturbing per-widget visibility.
#[derive(Component)]
pub struct GameHud;
pub fn mark_roots(mut commands:Commands,roots:Query<Entity,(With<Node>,Without<ChildOf>)>) {
    for e in &roots {commands.entity(e).insert(GameHud);}
}
pub fn menu_visibility(menu:Res<crate::menu::Menu>,mut roots:Query<&mut Node,With<GameHud>>) {
    let display=if menu.open(){Display::None}else{Display::Flex};
    for mut node in &mut roots {if node.display!=display {node.display=display;}}
}
