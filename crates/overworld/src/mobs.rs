//! Adapted from the pinned mashup Minecraft entity adapter; see third_party/README.md.
use minecraftoss_entities::world::{EntityWorld, MobHit};
pub fn boxes(world: &EntityWorld) -> Vec<(u64, [f64; 6])> {
    let w = world;
    let mut out = Vec::new();
    let mut add = |hit: MobHit, body: &minecraftoss_entities::movement::Body| {
        let p = body.position;
        let half = f64::from(body.width) * 0.5;
        out.push((
            encode(hit),
            [
                p.x - half,
                p.y,
                p.z - half,
                p.x + half,
                p.y + f64::from(body.height),
                p.z + half,
            ],
        ));
    };
    for e in w.bats().iter().filter(|e| e.bat.health > 0.0) {
        add(MobHit::Bat(e.id), &e.bat.body);
    }
    for e in w.zombies().iter().filter(|e| e.zombie.health > 0.0) {
        add(MobHit::Zombie(e.id), &e.zombie.body);
    }
    for e in w.skeletons().iter().filter(|e| e.skeleton.health > 0.0) {
        add(MobHit::Skeleton(e.id), &e.skeleton.body);
    }
    for e in w
        .creepers()
        .iter()
        .filter(|e| e.creeper.health > 0.0 && !e.creeper.exploded)
    {
        add(MobHit::Creeper(e.id), &e.creeper.body);
    }
    for e in w.spiders().iter().filter(|e| e.spider.health > 0.0) {
        add(MobHit::Spider(e.id), &e.spider.body);
    }
    for e in w.slimes().iter().filter(|e| e.slime.health > 0.0) {
        add(MobHit::Slime(e.id), &e.slime.body);
    }
    for e in w.endermen().iter().filter(|e| e.enderman.health > 0.0) {
        add(MobHit::Enderman(e.id), &e.enderman.body);
    }
    for e in w.witches().iter().filter(|e| e.witch.health > 0.0) {
        add(MobHit::Witch(e.id), &e.witch.body);
    }
    for e in w.iron_golems().iter().filter(|e| e.golem.health > 0.0) {
        add(MobHit::IronGolem(e.id), &e.golem.body);
    }
    for e in w.wolves().iter().filter(|e| e.wolf.health > 0.0) {
        add(MobHit::Wolf(e.id), &e.wolf.body);
    }
    for e in w.villagers().iter().filter(|e| e.villager.health > 0.0) {
        add(MobHit::Villager(e.id), &e.villager.body);
    }
    for e in w.cows().iter().filter(|e| e.cow.health > 0.0) {
        let hit = if e.mooshroom.is_some() {
            MobHit::Mooshroom(e.id)
        } else {
            MobHit::Cow(e.id)
        };
        add(hit, &e.cow.body);
    }
    for e in w.sheep().iter().filter(|e| e.health > 0.0) {
        add(MobHit::Sheep(e.id), &e.body);
    }
    for e in w.pigs().iter().filter(|e| e.pig.health > 0.0) {
        add(MobHit::Pig(e.id), &e.pig.body);
    }
    for e in w.chickens().iter().filter(|e| e.chicken.health > 0.0) {
        add(MobHit::Chicken(e.id), &e.chicken.body);
    }
    out
}
fn encode(hit: MobHit) -> u64 {
    let kind: u64 = match hit {
        MobHit::Bat(_) => 0,
        MobHit::Zombie(_) => 1,
        MobHit::Skeleton(_) => 2,
        MobHit::Creeper(_) => 3,
        MobHit::Spider(_) => 4,
        MobHit::Slime(_) => 5,
        MobHit::Enderman(_) => 6,
        MobHit::Witch(_) => 7,
        MobHit::IronGolem(_) => 8,
        MobHit::Wolf(_) => 9,
        MobHit::Villager(_) => 10,
        MobHit::Cow(_) => 11,
        MobHit::Mooshroom(_) => 12,
        MobHit::Sheep(_) => 13,
        MobHit::Pig(_) => 14,
        MobHit::Chicken(_) => 15,
    };
    (kind << 56) | (hit.id() & ((1 << 56) - 1))
}
pub fn decode(key: u64) -> Option<MobHit> {
    let id = key & ((1 << 56) - 1);
    Some(match key >> 56 {
        0 => MobHit::Bat(id),
        1 => MobHit::Zombie(id),
        2 => MobHit::Skeleton(id),
        3 => MobHit::Creeper(id),
        4 => MobHit::Spider(id),
        5 => MobHit::Slime(id),
        6 => MobHit::Enderman(id),
        7 => MobHit::Witch(id),
        8 => MobHit::IronGolem(id),
        9 => MobHit::Wolf(id),
        10 => MobHit::Villager(id),
        11 => MobHit::Cow(id),
        12 => MobHit::Mooshroom(id),
        13 => MobHit::Sheep(id),
        14 => MobHit::Pig(id),
        15 => MobHit::Chicken(id),
        _ => return None,
    })
}
