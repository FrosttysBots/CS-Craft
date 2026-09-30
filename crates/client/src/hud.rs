//! HUD: crosshair, health/armour, ammo, kill feed, hit marker, debug line.

use crate::effects::Effects;
use crate::player::{LocalPlayer, SimWorld};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

#[derive(Component)]
pub struct SpeedText;

#[derive(Component)]
pub struct DebugText;

#[derive(Component)]
pub struct HealthText;

#[derive(Component)]
pub struct AmmoText;

#[derive(Component)]
pub struct FeedText;

#[derive(Component)]
pub struct CenterText;

#[derive(Component)]
pub struct HitMarker;

const HUD_YELLOW: Color = Color::srgb(0.95, 0.85, 0.55);

fn text_node(commands: &mut Commands, size: f32, color: Color, node: Node) -> Entity {
    commands
        .spawn((
            Text::new(""),
            TextFont {
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
            node,
        ))
        .id()
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
                for (x, y, w, h) in bars {
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

    let speed = text_node(
        &mut commands,
        20.0,
        Color::srgba(1.0, 1.0, 1.0, 0.8),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(90.0),
            width: percent(100),
            justify_content: JustifyContent::Center,
            ..default()
        },
    );
    commands.entity(speed).insert((SpeedText, TextLayout::justify(Justify::Center)));

    let debug = text_node(
        &mut commands,
        14.0,
        Color::srgba(1.0, 1.0, 1.0, 0.85),
        Node {
            position_type: PositionType::Absolute,
            top: px(6.0),
            left: px(8.0),
            ..default()
        },
    );
    commands.entity(debug).insert(DebugText);

    let health = text_node(
        &mut commands,
        34.0,
        HUD_YELLOW,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(16.0),
            left: px(24.0),
            ..default()
        },
    );
    commands.entity(health).insert(HealthText);

    let ammo = text_node(
        &mut commands,
        34.0,
        HUD_YELLOW,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(16.0),
            right: px(24.0),
            ..default()
        },
    );
    commands.entity(ammo).insert((AmmoText, TextLayout::justify(Justify::Right)));

    let feed = text_node(
        &mut commands,
        16.0,
        Color::WHITE,
        Node {
            position_type: PositionType::Absolute,
            top: px(60.0),
            right: px(16.0),
            ..default()
        },
    );
    commands.entity(feed).insert((FeedText, TextLayout::justify(Justify::Right)));

    let center = text_node(
        &mut commands,
        28.0,
        Color::srgb(1.0, 0.35, 0.3),
        Node {
            position_type: PositionType::Absolute,
            top: percent(30),
            width: percent(100),
            justify_content: JustifyContent::Center,
            ..default()
        },
    );
    commands.entity(center).insert((CenterText, TextLayout::justify(Justify::Center)));
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn update(
    local: Res<LocalPlayer>,
    world: Res<SimWorld>,
    effects: Res<Effects>,
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    level_name: Res<crate::LevelInfo>,
    mut texts: ParamSet<(
        Single<&mut Text, With<SpeedText>>,
        Single<&mut Text, With<DebugText>>,
        Single<&mut Text, With<HealthText>>,
        Single<&mut Text, With<AmmoText>>,
        Single<&mut Text, With<FeedText>>,
        Single<&mut Text, With<CenterText>>,
    )>,
    mut markers: Query<&mut Visibility, With<HitMarker>>,
) {
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let Some(p) = world.0.player(local.id) else { return };
    let now = time.elapsed_secs();

    texts.p0().0 = format!("{:.0}", p.movement.velocity.truncate().length());

    let o = local.curr_eye;
    let t_score: u32 = world.0.players.iter().filter(|q| q.team == sim::Team::Terrorist).map(|q| q.kills).sum();
    let ct_score: u32 = world.0.players.iter().filter(|q| q.team == sim::Team::CounterTerrorist).map(|q| q.kills).sum();
    texts.p1().0 = format!(
        "CSGO Rewrite | {} | {:.0} fps | pos {:.0} {:.0} {:.0} | T {t_score} - CT {ct_score} | K {} D {}{}\nclick to capture mouse | WASD move | space jump | ctrl crouch | shift walk | LMB fire | R reload | 1-3/wheel weapons | V noclip | Esc release",
        level_name.name,
        fps,
        o.x,
        o.y,
        o.z,
        p.kills,
        p.deaths,
        if local.noclip { " | NOCLIP" } else { "" },
    );

    texts.p2().0 = if p.alive { format!("HP {}   AR {}", p.health, p.armor) } else { String::new() };

    texts.p3().0 = match p.active_weapon() {
        Some(w) if p.alive => {
            let name = w.def.class_name.trim_start_matches("weapon_").to_uppercase();
            if w.uses_ammo() {
                let reload = if w.reload_end.is_some() { "  RELOADING" } else { "" };
                format!("{name}{reload}\n{} / {}", w.clip, w.reserve)
            } else {
                name
            }
        }
        _ => String::new(),
    };

    texts.p4().0 = effects
        .kill_feed
        .iter()
        .map(|k| if k.local_involved { format!("> {}", k.text) } else { k.text.clone() })
        .collect::<Vec<_>>()
        .join("\n");

    texts.p5().0 = if !p.alive && !local.noclip {
        let wait = (p.respawn_at - world.0.time()).max(0.0);
        format!("YOU DIED - respawning in {wait:.1}s")
    } else {
        String::new()
    };

    let show = now - effects.last_hit < 0.12;
    for mut v in &mut markers {
        *v = if show { Visibility::Inherited } else { Visibility::Hidden };
    }
}
