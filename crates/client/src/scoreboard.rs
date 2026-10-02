//! Hold Tab for the current teams and match score. Only teammates' money
//! and equipment are exposed; enemy loadouts stay private.

use crate::buymenu::BuyMenu;
use crate::player::{InputEnabled, LocalPlayer, SimWorld};
use crate::LevelInfo;
use bevy::prelude::*;
use sim::{Phase, PlayerId, Team};

#[derive(Resource, Default)]
pub struct Scoreboard {
    root: Option<Entity>,
    shown: Option<Board>,
}

#[derive(PartialEq)]
struct Board {
    title: String,
    teams: Vec<(String, Vec<Row>)>,
}

#[derive(PartialEq)]
struct Row {
    id: PlayerId,
    name: String,
    status: String,
    money: String,
    kills: u32,
    deaths: u32,
}

fn board(world: &sim::World, local: PlayerId, map: &str) -> Board {
    let team = world.player(local).map_or(Team::Spectator, |p| p.team);
    let competitive = world.competitive();
    let title = if competitive {
        format!("{map}  /  COMPETITIVE  /  ROUND {}", world.round.round)
    } else if world.mode == sim::GameMode::Special {
        format!("{map}  /  SPECIAL  /  SOLO  /  UNLIMITED TIME")
    } else {
        format!("{map}  /  DEATHMATCH")
    };
    let teams = [(Team::CounterTerrorist, "COUNTER-TERRORISTS"), (Team::Terrorist, "TERRORISTS")]
        .into_iter()
        .map(|(side, name)| {
            let score = if competitive {
                world.round.score(side)
            } else {
                world.players.iter().filter(|p| p.team == side).map(|p| p.kills).sum()
            };
            let mut rows: Vec<Row> = world.players.iter().filter(|p| p.team == side).map(|p| {
                let own = p.team == team;
                let status = if !p.alive { "DEAD" } else if own && p.has_bomb() { "C4" } else if own && p.defuse_kit { "KIT" } else { "ALIVE" };
                Row {
                    id: p.id,
                    name: format!("{}{}", p.name, if p.id == local { " (you)" } else { "" }),
                    status: status.into(),
                    money: if own && competitive { format!("${}", p.money) } else { String::new() },
                    kills: p.kills,
                    deaths: p.deaths,
                }
            }).collect();
            rows.sort_by_key(|r| (std::cmp::Reverse(r.kills), r.deaths, r.id));
            (format!("{name}    {score}"), rows)
        }).collect();
    Board { title, teams }
}

fn label(parent: &mut ChildSpawnerCommands, value: impl Into<String>, size: f32, color: Color) {
    parent.spawn((Text::new(value), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color)));
}

fn cell(parent: &mut ChildSpawnerCommands, value: impl Into<String>, width: Val, color: Color) {
    parent.spawn((
        Text::new(value),
        TextFont { font_size: FontSize::Px(17.0), ..default() },
        TextColor(color),
        Node { width, flex_shrink: 0.0, ..default() },
    ));
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut commands: Commands,
    mut state: ResMut<Scoreboard>,
    world: Res<SimWorld>,
    local: Res<LocalPlayer>,
    level: Res<LevelInfo>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<crate::settings::Settings>,
    input: Res<InputEnabled>,
    menu: Res<BuyMenu>,
    art: Res<crate::ui_assets::UiAssets>,
) {
    let preview = !input.0 && std::env::var_os("CSR_SCOREBOARD").is_some();
    let show = !menu.open && (preview || (input.0 && keys.pressed(settings.binds[11])) || (world.0.competitive() && world.0.round.phase == Phase::MatchOver));
    let next = show.then(|| board(&world.0, local.id, &level.name));
    if state.shown == next {
        return;
    }
    if let Some(root) = state.root.take() {
        commands.entity(root).despawn();
    }
    state.shown = next;
    let Some(board) = &state.shown else { return };
    let root = commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        GlobalZIndex(20),
    )).with_children(|screen| {
        screen.spawn((
            Node {
                width: px(900.0),
                max_width: percent(94),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(20.0)),
                row_gap: px(6.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.07, 0.08, 0.09, 0.94)),
        )).with_children(|panel| {
            label(panel, &board.title, 20.0, Color::WHITE);
            for (index, (title, rows)) in board.teams.iter().enumerate() {
                let color = if index == 0 { Color::srgb(0.45, 0.7, 1.0) } else { Color::srgb(0.95, 0.75, 0.3) };
                panel.spawn(Node { align_items: AlignItems::Center, column_gap: px(12.0), padding: UiRect::vertical(px(8.0)), ..default() }).with_children(|heading| {
                    if let Some(image) = art.image(if index == 0 { "ct" } else { "t" }) { heading.spawn((ImageNode::new(image), Node { width: px(38.0), height: px(38.0), ..default() })); }
                    label(heading, title, 24.0, color);
                });
                panel.spawn(Node { width: percent(100), padding: UiRect::axes(px(8.0), px(2.0)), ..default() }).with_children(|header| {
                    for (title, width) in [("PLAYER", 48), ("STATUS", 18), ("MONEY", 18), ("K", 8), ("D", 8)] {
                        cell(header, title, percent(width), Color::srgb(0.55, 0.6, 0.65));
                    }
                });
                for row in rows {
                    let color = if row.status == "DEAD" { Color::srgb(0.55, 0.6, 0.65) } else { Color::WHITE };
                    panel.spawn((
                        Node { width: percent(100), padding: UiRect::axes(px(8.0), px(3.0)), ..default() },
                        BackgroundColor(if row.id == local.id { Color::srgba(1.0, 1.0, 1.0, 0.12) } else { Color::NONE }),
                    )).with_children(|line| {
                        cell(line, &row.name, percent(48), color);
                        cell(line, &row.status, percent(18), color);
                        cell(line, &row.money, percent(18), color);
                        cell(line, row.kills.to_string(), percent(8), color);
                        cell(line, row.deaths.to_string(), percent(8), color);
                    });
                }
            }
            label(panel, "Hold TAB to view  /  K: kills  /  D: deaths", 14.0, Color::srgb(0.55, 0.6, 0.65));
        });
    }).id();
    state.root = Some(root);
}
