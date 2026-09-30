//! Minimal HUD: classic crosshair, speed meter and a debug line.

use crate::player::{LocalPlayer, SimWorld};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

#[derive(Component)]
pub struct SpeedText;

#[derive(Component)]
pub struct DebugText;

pub fn setup(mut commands: Commands) {
    // Crosshair: four green bars with a gap, CS:GO "classic static" style.
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
            });
        });

    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(22.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 1.0, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(90.0),
            width: percent(100),
            justify_content: JustifyContent::Center,
            ..default()
        },
        TextLayout::justify(Justify::Center),
        SpeedText,
    ));
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            top: px(6.0),
            left: px(8.0),
            ..default()
        },
        DebugText,
    ));
}

pub fn update(
    local: Res<LocalPlayer>,
    world: Res<SimWorld>,
    diagnostics: Res<DiagnosticsStore>,
    mut speed: Single<&mut Text, (With<SpeedText>, Without<DebugText>)>,
    mut debug: Single<&mut Text, (With<DebugText>, Without<SpeedText>)>,
    level_name: Res<crate::LevelInfo>,
) {
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let Some(p) = world.0.player(local.id) else { return };
    let v = p.movement.velocity;
    speed.0 = format!("{:.0}", v.truncate().length());
    let o = local.curr_eye;
    debug.0 = format!(
        "CSGO Rewrite | {} | {:.0} fps | pos {:.0} {:.0} {:.0} | ang {:.1} {:.1}{}{}\nclick to capture mouse | WASD move | space jump | ctrl crouch | shift walk | V noclip | Esc release",
        level_name.name,
        fps,
        o.x,
        o.y,
        o.z,
        local.pitch,
        local.yaw,
        if local.noclip { " | NOCLIP" } else { "" },
        if p.movement.on_ground { "" } else { " | air" },
    );
}
