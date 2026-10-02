//! Replay the same authored goal inputs against Rust and compare Java events.
use minecraftoss_entities::goals::{Control, Controls, Goal, GoalSelector};
use serde_json::{json, Value};
use std::{env, fs};

struct Context {
    enabled: Vec<bool>,
    events: Vec<String>,
}
#[derive(Clone)]
struct Probe {
    id: usize,
    flags: Controls,
    every: bool,
    interruptible: bool,
}
impl Goal<Context> for Probe {
    fn controls(&self) -> Controls {
        self.flags
    }
    fn can_start(&mut self, c: &mut Context, _world: &dyn minecraftoss_player::World) -> bool {
        c.events.push(format!("use{}", self.id));
        c.enabled[self.id]
    }
    fn can_continue(&mut self, c: &mut Context, _world: &dyn minecraftoss_player::World) -> bool {
        c.events.push(format!("continue{}", self.id));
        c.enabled[self.id]
    }
    fn start(&mut self, c: &mut Context) {
        c.events.push(format!("start{}", self.id));
    }
    fn stop(&mut self, c: &mut Context) {
        c.events.push(format!("stop{}", self.id));
    }
    fn tick(&mut self, c: &mut Context) {
        c.events.push(format!("tick{}", self.id));
    }
    fn every_tick(&self) -> bool {
        self.every
    }
    fn interruptible(&self) -> bool {
        self.interruptible
    }
}
struct NoWorld;
impl minecraftoss_player::World for NoWorld {
    fn block(&self, _pos: minecraftoss_player::Pos) -> Option<minecraftoss_player::Block> {
        None
    }
    fn set_block(&mut self, _pos: minecraftoss_player::Pos, _block: Option<minecraftoss_player::Block>) {}
}
fn control(v: &Value) -> Control {
    match v.as_str().unwrap() {
        "MOVE" => Control::Move,
        "LOOK" => Control::Look,
        "JUMP" => Control::Jump,
        "TARGET" => Control::Target,
        _ => panic!("unknown flag"),
    }
}
fn main() {
    let args: Vec<_> = env::args().collect();
    assert_eq!(
        args.len(),
        4,
        "usage: check_goal_selector INPUT REFERENCE_A REFERENCE_B"
    );
    let read =
        |path: &str| serde_json::from_str::<Value>(&fs::read_to_string(path).unwrap()).unwrap();
    let input = read(&args[1]);
    let expected = read(&args[2]);
    assert_eq!(
        expected,
        read(&args[3]),
        "Java references must repeat exactly"
    );
    let mut output = Vec::new();
    let mut count = 0;
    for case in input.as_array().unwrap() {
        let mut selector = GoalSelector::default();
        for (id, g) in case["goals"].as_array().unwrap().iter().enumerate() {
            selector.add(
                g["priority"].as_i64().unwrap() as i32,
                Probe {
                    id,
                    flags: Controls::new(
                        &g["flags"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(control)
                            .collect::<Vec<_>>(),
                    ),
                    every: g["every_tick"].as_bool().unwrap(),
                    interruptible: g["interruptible"].as_bool().unwrap(),
                },
            );
        }
        let mut phases = Vec::new();
        for phase in case["phases"].as_array().unwrap() {
            let mut context = Context {
                enabled: phase["enabled"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_bool().unwrap())
                    .collect(),
                events: Vec::new(),
            };
            for flag in [Control::Move, Control::Look, Control::Jump, Control::Target] {
                selector.set_enabled(flag, true);
            }
            for flag in phase["disabled"].as_array().unwrap() {
                selector.set_enabled(control(flag), false);
            }
            if phase["full"].as_bool().unwrap() {
                selector.tick(&mut context, &NoWorld);
            } else {
                selector.tick_running(&mut context, false);
            }
            phases.push(context.events);
            count += 1;
        }
        output.push(json!({"id":case["id"],"events":phases}));
    }
    assert_eq!(json!(output), expected);
    println!("{count} goal arbitration phases match repeated pinned Java events exactly");
}
