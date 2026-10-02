//! Local cosmetic inventory. Valve content is read from the installation, never exported.
use bevy::prelude::*;
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default)]
pub struct Finish {
    pub id: u32,
    pub weapon: String,
    pub name: String,
    pub paint: String,
    pub rarity: u8,
    pub icon: String,
    pub style: u32,
    pub pattern: String,
    pub colors: [Vec3; 4],
    pub wear: [f32; 2],
    pub scale: f32,
    pub offset: [f32; 4],
    pub rotation: [f32; 2],
    pub only_first_material: bool,
    pub glove_material: String,
    pub phong: [f32; 2],
    pub albedo_boost: f32,
}
#[derive(Clone, Debug, Default)]
pub struct Case {
    pub id: u32,
    pub key: String,
    pub name: String,
    pub icon: String,
    pub contents: Vec<usize>,
    pub souvenir: bool,
    pub event: u32,
}
#[derive(Resource, Default)]
pub struct Catalog {
    pub cases: Vec<Case>,
    pub finishes: Vec<Finish>,
    pub models: BTreeMap<String, ModelInfo>,
    pub stickers: BTreeMap<u32, StickerDefinition>,
    pub materials: BTreeMap<String, Vec<(String, String, String)>>,
}
pub fn is_glove(name: &str) -> bool {
    name.contains("gloves") || name == "leather_handwraps"
}
pub fn is_rare_knife(name: &str) -> bool {
    matches!(
        name,
        "weapon_bayonet"
            | "weapon_knife_css"
            | "weapon_knife_flip"
            | "weapon_knife_gut"
            | "weapon_knife_karambit"
            | "weapon_knife_m9_bayonet"
            | "weapon_knife_tactical"
            | "weapon_knife_falchion"
            | "weapon_knife_survival_bowie"
            | "weapon_knife_butterfly"
            | "weapon_knife_push"
            | "weapon_knife_cord"
            | "weapon_knife_canis"
            | "weapon_knife_ursus"
            | "weapon_knife_gypsy_jackknife"
            | "weapon_knife_outdoor"
            | "weapon_knife_stiletto"
            | "weapon_knife_widowmaker"
            | "weapon_knife_skeleton"
    )
}
pub fn equip_slot(name: &str) -> &str {
    if is_glove(name) {
        "gloves"
    } else if name == "weapon_knife" || is_rare_knife(name) || name == "weapon_knife_t" {
        "knife"
    } else {
        name
    }
}
#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub view: String,
    pub world: String,
    pub stickers: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct StickerDefinition {
    pub name: String,
    pub material: String,
    pub event: u32,
    pub team: u32,
    pub player: u32,
}
impl Catalog {
    pub fn load(fs: &vfs::Vfs) -> Result<Self, String> {
        crate::cosmetic_catalog::load(fs)
    }
    pub fn find_case(&self, query: &str) -> Result<&Case, String> {
        let q = query.trim().trim_matches('"').to_lowercase();
        if q.is_empty() {
            return Err("Usage: give_case <case name or ID> [count]".into());
        }
        if let Some(c) = self
            .cases
            .iter()
            .find(|c| c.id.to_string() == q || c.key == q || c.name.to_lowercase() == q)
        {
            return Ok(c);
        }
        let matches: Vec<_> = self
            .cases
            .iter()
            .filter(|c| c.name.to_lowercase().contains(&q) || c.key.contains(&q))
            .collect();
        match matches.as_slice() {
            [c] => Ok(c),
            [] => Err(format!("Unknown case: {query}. Use cases to list names.")),
            _ => Err(format!(
                "Ambiguous case: {}",
                matches
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
    pub fn finish(&self, item: &OwnedSkin) -> Option<&Finish> {
        self.finishes
            .iter()
            .find(|f| f.id == item.paint && f.weapon == item.weapon)
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OwnedSkin {
    pub serial: u64,
    pub weapon: String,
    pub paint: u32,
    pub seed: u32,
    pub wear: f32,
    pub stattrak: Option<u32>,
    pub souvenir: Option<u32>,
    pub stickers: [Option<StickerSlot>; 4],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickerSlot {
    pub kit: u32,
    pub wear: f32,
}
impl OwnedSkin {
    pub fn display_name(&self, finish: &Finish) -> String {
        format!(
            "{}{}",
            if self.stattrak.is_some() {
                "StatTrak™ "
            } else if self.souvenir.is_some() {
                "Souvenir "
            } else {
                ""
            },
            finish.name
        )
    }
    pub fn condition(&self) -> &'static str {
        match self.wear {
            x if x < 0.07 => "Factory New",
            x if x < 0.15 => "Minimal Wear",
            x if x < 0.38 => "Field-Tested",
            x if x < 0.45 => "Well-Worn",
            _ => "Battle-Scarred",
        }
    }
}
#[derive(Resource, Clone, Default, Debug, PartialEq)]
pub struct Inventory {
    pub cases: BTreeMap<u32, u32>,
    pub skins: Vec<OwnedSkin>,
    pub equipped: BTreeMap<String, u64>,
    pub next: u64,
    pub sticker_stock: BTreeMap<u32, u32>,
}
pub fn profile_path() -> PathBuf {
    std::env::var_os("CSR_PROFILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config"))
        .join("cosmetics.cfg")
}
impl Inventory {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::decode(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                next: 1,
                ..default()
            }),
            Err(e) => Err(e.to_string()),
        }
    }
    fn decode(text: &str) -> Result<Self, String> {
        let mut out = Self {
            next: 1,
            ..default()
        };
        if !matches!(
            text.lines().next(),
            Some("CSR-COSMETICS 1" | "CSR-COSMETICS 2")
        ) {
            return Err("Unrecognized cosmetics profile; file left untouched.".into());
        }
        for line in text.lines().skip(1) {
            let p: Vec<_> = line.split_whitespace().collect();
            let invalid = || format!("Invalid cosmetics record: {line}");
            match p.as_slice() {
                ["case", id, n] => {
                    let id = id.parse().map_err(|_| invalid())?;
                    let n = n.parse().map_err(|_| invalid())?;
                    if out.cases.insert(id, n).is_some() {
                        return Err(invalid());
                    }
                }
                ["skin", serial, weapon, paint, seed, wear] => {
                    let item = OwnedSkin {
                        serial: serial.parse().map_err(|_| invalid())?,
                        weapon: weapon.to_string(),
                        paint: paint.parse().map_err(|_| invalid())?,
                        seed: seed.parse().map_err(|_| invalid())?,
                        wear: wear.parse().map_err(|_| invalid())?,
                        ..default()
                    };
                    if item.serial == 0
                        || item.serial == u64::MAX
                        || item.seed > 1000
                        || !item.wear.is_finite()
                        || !(0.0..=1.0).contains(&item.wear)
                        || out.skins.iter().any(|s| s.serial == item.serial)
                    {
                        return Err(invalid());
                    }
                    out.next = out.next.max(item.serial + 1);
                    out.skins.push(item);
                }
                ["equip", weapon, serial] => {
                    out.equipped
                        .insert(weapon.to_string(), serial.parse().map_err(|_| invalid())?);
                }
                ["quality", serial, stat, souvenir] => {
                    let id = serial.parse::<u64>().map_err(|_| invalid())?;
                    let item = out
                        .skins
                        .iter_mut()
                        .find(|s| s.serial == id)
                        .ok_or_else(invalid)?;
                    item.stattrak = if *stat == "-" {
                        None
                    } else {
                        Some(stat.parse::<u32>().map_err(|_| invalid())?)
                    };
                    item.souvenir = if *souvenir == "-" {
                        None
                    } else {
                        Some(souvenir.parse::<u32>().map_err(|_| invalid())?)
                    };
                    if (item.stattrak.is_some()
                        && (item.souvenir.is_some() || is_glove(&item.weapon)))
                        || (item.souvenir.is_some()
                            && (is_rare_knife(&item.weapon) || is_glove(&item.weapon)))
                    {
                        return Err(invalid());
                    }
                }
                ["sticker", serial, slot, kit, wear] => {
                    let id = serial.parse::<u64>().map_err(|_| invalid())?;
                    let slot = slot.parse::<usize>().map_err(|_| invalid())?;
                    let wear = wear.parse::<f32>().map_err(|_| invalid())?;
                    if slot >= 4 || !wear.is_finite() || !(0.0..1.0).contains(&wear) {
                        return Err(invalid());
                    }
                    let item = out
                        .skins
                        .iter_mut()
                        .find(|s| s.serial == id)
                        .ok_or_else(invalid)?;
                    if item.stickers[slot].is_some() {
                        return Err(invalid());
                    }
                    item.stickers[slot] = Some(StickerSlot {
                        kit: kit.parse().map_err(|_| invalid())?,
                        wear,
                    });
                }
                ["sticker_stock", kit, count] => {
                    out.sticker_stock.insert(
                        kit.parse().map_err(|_| invalid())?,
                        count.parse().map_err(|_| invalid())?,
                    );
                }
                [] => {}
                _ => return Err(invalid()),
            }
        }
        if out.equipped.iter().any(|(w, id)| {
            !out.skins
                .iter()
                .any(|s| s.serial == *id && equip_slot(&s.weapon) == w)
        }) {
            return Err("Equipped skin is not owned".into());
        }
        Ok(out)
    }
    fn encode(&self) -> String {
        let mut s = String::from("CSR-COSMETICS 2\n");
        for (id, n) in &self.cases {
            s.push_str(&format!("case {id} {n}\n"));
        }
        for i in &self.skins {
            s.push_str(&format!(
                "skin {} {} {} {} {}\n",
                i.serial, i.weapon, i.paint, i.seed, i.wear
            ));
            if i.stattrak.is_some() || i.souvenir.is_some() {
                s.push_str(&format!(
                    "quality {} {} {}\n",
                    i.serial,
                    i.stattrak.map_or("-".into(), |n| n.to_string()),
                    i.souvenir.map_or("-".into(), |n| n.to_string())
                ));
            }
            for (slot, sticker) in i.stickers.iter().enumerate() {
                if let Some(sticker) = sticker {
                    s.push_str(&format!(
                        "sticker {} {slot} {} {}\n",
                        i.serial, sticker.kit, sticker.wear
                    ));
                }
            }
        }
        for (w, id) in &self.equipped {
            s.push_str(&format!("equip {w} {id}\n"));
        }
        for (id, n) in &self.sticker_stock {
            s.push_str(&format!("sticker_stock {id} {n}\n"));
        }
        s
    }
    /// Reload under a cross-process lock so console/CLI grants cannot overwrite each other.
    pub fn transaction<T>(
        &mut self,
        path: &Path,
        change: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let parent = path.parent().ok_or("Invalid inventory path")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let lock = path.with_extension("lock");
        let guard = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock)
            .map_err(|e| e.to_string())?;
        guard
            .try_lock()
            .map_err(|e| format!("Inventory is busy ({}): {e}", lock.display()))?;
        let result = (|| {
            let mut next = Self::load(path)?;
            let value = change(&mut next)?;
            let temp = path.with_extension("tmp");
            let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
            file.write_all(next.encode().as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            drop(file);
            std::fs::rename(&temp, path).map_err(|e| e.to_string())?;
            *self = next;
            Ok(value)
        })();
        drop(guard);
        result
    }
    pub fn grant(&mut self, id: u32, count: u32) -> Result<(), String> {
        if !(1..=1000).contains(&count) {
            return Err("Count must be 1–1000.".into());
        }
        let n = self.cases.entry(id).or_default();
        *n = n.checked_add(count).ok_or("Case count overflow")?;
        Ok(())
    }
    pub fn open(
        &mut self,
        case: &Case,
        catalog: &Catalog,
        rng: &mut Random,
    ) -> Result<OwnedSkin, String> {
        if self.cases.get(&case.id).copied().unwrap_or(0) == 0 {
            return Err("You do not own this case. Use give_case to add one.".into());
        }
        let finish = roll(case, catalog, rng).ok_or("This case has no supported contents")?;
        let f = &catalog.finishes[finish];
        let serial = self.next.max(1);
        let next = serial.checked_add(1).ok_or("Inventory ID overflow")?;
        let item = OwnedSkin {
            serial,
            weapon: f.weapon.clone(),
            paint: f.id,
            seed: (rng.unit() * 1001.) as u32,
            wear: f.wear[0] + rng.unit() * (f.wear[1] - f.wear[0]).max(0.),
            stattrak: (!case.souvenir && !is_glove(&f.weapon) && rng.unit() < 0.1).then_some(0),
            souvenir: case.souvenir.then_some(case.event),
            stickers: if case.souvenir {
                souvenir_stickers(catalog, case.event, &case.key, rng)
            } else {
                [None; 4]
            },
        };
        *self.cases.get_mut(&case.id).unwrap() -= 1;
        self.next = next;
        self.skins.push(item.clone());
        Ok(item)
    }
    pub fn equip(&mut self, serial: u64) -> Result<(), String> {
        let i = self
            .skins
            .iter()
            .find(|s| s.serial == serial)
            .ok_or("Skin no longer owned")?;
        self.equipped
            .insert(equip_slot(&i.weapon).to_string(), serial);
        Ok(())
    }
    pub fn active(&self, weapon: &str) -> Option<&OwnedSkin> {
        let id = self.equipped.get(equip_slot(weapon))?;
        self.skins.iter().find(|s| s.serial == *id)
    }
    pub fn apply_sticker(
        &mut self,
        serial: u64,
        slot: usize,
        kit: u32,
        catalog: &Catalog,
    ) -> Result<(), String> {
        if !catalog.stickers.contains_key(&kit) {
            return Err("Unknown sticker kit".into());
        }
        let item = self
            .skins
            .iter_mut()
            .find(|s| s.serial == serial)
            .ok_or("Item not owned")?;
        let slots = catalog
            .models
            .get(&item.weapon)
            .map_or(0, |m| m.stickers.len().min(4));
        if slot >= slots {
            return Err("This weapon has no sticker position there".into());
        }
        if item.stickers[slot].is_some() {
            return Err("Scrape off the existing sticker first".into());
        }
        let count = self
            .sticker_stock
            .get_mut(&kit)
            .filter(|n| **n > 0)
            .ok_or("Sticker not owned; use give_sticker")?;
        *count -= 1;
        item.stickers[slot] = Some(StickerSlot { kit, wear: 0. });
        Ok(())
    }
    pub fn scrape(&mut self, serial: u64, slot: usize) -> Result<(), String> {
        let item = self
            .skins
            .iter_mut()
            .find(|s| s.serial == serial)
            .ok_or("Item not owned")?;
        let target = item
            .stickers
            .get_mut(slot)
            .ok_or("Sticker slot must be 1–4")?;
        let sticker = target.as_mut().ok_or("No sticker in that slot")?;
        sticker.wear += 0.2;
        if sticker.wear >= 0.99 {
            *target = None;
        }
        Ok(())
    }
}
fn souvenir_stickers(
    catalog: &Catalog,
    event: u32,
    package: &str,
    rng: &mut Random,
) -> [Option<StickerSlot>; 4] {
    let mut kits: Vec<_> = catalog
        .stickers
        .iter()
        .filter(|(_, s)| s.event == event && s.name.to_lowercase().contains("(gold)"))
        .collect();
    if kits.is_empty() {
        kits = catalog
            .stickers
            .iter()
            .filter(|(_, s)| s.event == event && s.name.to_lowercase().contains("(foil)"))
            .collect();
    }
    if kits.is_empty() {
        kits = catalog
            .stickers
            .iter()
            .filter(|(_, s)| s.event == event)
            .collect();
    }
    let mut choose = |list: Vec<_>| -> Option<u32> {
        if list.is_empty() {
            None
        } else {
            Some(list[(rng.unit() * list.len() as f32) as usize % list.len()])
        }
    };
    let tournament = choose(
        kits.iter()
            .filter(|(_, s)| s.team == 0 && s.player == 0)
            .map(|(id, _)| **id)
            .collect(),
    );
    let team1 = choose(
        kits.iter()
            .filter(|(_, s)| s.team > 0 && s.player == 0)
            .map(|(id, _)| **id)
            .collect(),
    );
    let first_team = team1
        .and_then(|id| catalog.stickers.get(&id))
        .map_or(0, |s| s.team);
    let team2 = choose(
        kits.iter()
            .filter(|(_, s)| s.team > 0 && s.team != first_team && s.player == 0)
            .map(|(id, _)| **id)
            .collect(),
    );
    let fourth = if event >= 18 {
        let map = [
            "dust2",
            "mirage",
            "inferno",
            "nuke",
            "overpass",
            "vertigo",
            "ancient",
            "anubis",
            "train",
            "cache",
            "cobblestone",
            "dust",
        ]
        .into_iter()
        .find(|m| package.contains(&format!("de_{m}")));
        map.and_then(|m| {
            catalog
                .stickers
                .iter()
                .find(|(_, s)| s.material.ends_with(&format!("/de_{m}_gold")))
                .map(|(&id, _)| id)
        })
    } else {
        let second_team = team2
            .and_then(|id| catalog.stickers.get(&id))
            .map_or(0, |s| s.team);
        choose(
            kits.iter()
                .filter(|(_, s)| s.player > 0 && (s.team == first_team || s.team == second_team))
                .map(|(id, _)| **id)
                .collect(),
        )
    };
    [tournament, team1, team2, fourth].map(|id| id.map(|kit| StickerSlot { kit, wear: 0. }))
}
/// Local rarity weights, including rare-special items, normalized per container.
pub fn roll(case: &Case, catalog: &Catalog, rng: &mut Random) -> Option<usize> {
    let mut groups: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
    for &i in &case.contents {
        groups
            .entry(catalog.finishes[i].rarity)
            .or_default()
            .push(i);
    }
    let weight = |r: u8| {
        if r == 7 {
            0.4
        } else {
            5.0_f32.powi(6 - r as i32)
        }
    };
    let mut x = rng.unit() * groups.keys().map(|&r| weight(r)).sum::<f32>();
    for (r, items) in &groups {
        x -= weight(*r);
        if x <= 0. {
            return Some(items[(rng.unit() * items.len() as f32) as usize % items.len()]);
        }
    }
    case.contents.last().copied()
}
pub struct Random(pub u64);
impl Random {
    pub fn fresh() -> Self {
        Self(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
                ^ (std::process::id() as u64) << 32,
        )
    }
    pub fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        ((z ^ (z >> 31)) >> 40) as f32 / 16777216.
    }
}
pub fn command(
    line: &str,
    catalog: &Catalog,
    inventory: &mut Inventory,
    path: &Path,
) -> Result<String, String> {
    let line = line.trim();
    let (cmd, args) = line.split_once(' ').unwrap_or((line, ""));
    match cmd {
        "give_sticker"|"give-sticker"=>{
            let parts:Vec<_>=args.split_whitespace().collect();let id=parts.first().ok_or("Usage: give_sticker <kit ID> [count]")?.parse::<u32>().map_err(|_|"Sticker ID must be a number")?;
            let count=parts.get(1).unwrap_or(&"1").parse::<u32>().map_err(|_|"Invalid count")?;
            if !(1..=1000).contains(&count){return Err("Count must be 1–1000".into());}
            let s=catalog.stickers.get(&id).ok_or("Unknown sticker ID; use stickers <search>")?;
            inventory.transaction(path,|i|{let n=i.sticker_stock.entry(id).or_default();*n=n.checked_add(count).ok_or("Count overflow")?;Ok(())})?;
            Ok(format!("Added {count} × {}",s.name))
        }
        "stickers"=>Ok(catalog.stickers.iter().filter(|(_,s)|s.name.to_lowercase().contains(&args.to_lowercase())).take(50).map(|(id,s)|format!("{id}: {}",s.name)).collect::<Vec<_>>().join("\n")),
        "skins"=>Ok(catalog.finishes.iter().filter(|f|f.name.to_lowercase().contains(&args.to_lowercase())).take(50).map(|f|format!("{} {}: {}",f.weapon,f.id,f.name)).collect::<Vec<_>>().join("\n")),
        "give_skin"|"give-skin"=>{
            let p:Vec<_>=args.split_whitespace().collect();if p.len()<2{return Err("Usage: give_skin <weapon class> <paint ID> [normal|stattrak|souvenir]".into());}
            let paint=p[1].parse::<u32>().map_err(|_|"Invalid paint ID")?;
            let f=catalog.finishes.iter().find(|f|f.weapon==p[0] && f.id==paint).ok_or("Weapon/paint combination not available")?;
            let quality=p.get(2).copied().unwrap_or("normal");
            if !["normal","stattrak","souvenir"].contains(&quality) || (is_glove(&f.weapon)&&quality!="normal") || (is_rare_knife(&f.weapon)&&quality=="souvenir"){return Err("Invalid quality for this item".into());}
            let containing=|souvenir:bool|catalog.cases.iter().filter(|c|c.souvenir==souvenir).find(|c|c.contents.iter().any(|&n|catalog.finishes[n].weapon==f.weapon&&catalog.finishes[n].id==f.id));
            if quality=="stattrak" && containing(false).is_none(){return Err("This finish has no StatTrak case variant".into());}
            let event=if quality=="souvenir"{Some(containing(true).ok_or("This finish has no souvenir package variant")?.event)}else{None};
            inventory.transaction(path,|i|{
                let serial=i.next.max(1);i.next=serial.checked_add(1).ok_or("Item ID overflow")?;
                let mut rng=Random::fresh();
                i.skins.push(OwnedSkin{serial,weapon:f.weapon.clone(),paint:f.id,seed:((rng.unit()*1000.) as u32),wear:f.wear[0],stattrak:(quality=="stattrak").then_some(0),souvenir:event,stickers:event.map_or([None;4],|e|souvenir_stickers(catalog,e,&containing(true).unwrap().key,&mut rng))});Ok(())
            })?;Ok(format!("Added {quality} {}",f.name))
        }
        "cases" => Ok(catalog
            .cases
            .iter()
            .map(|c| format!("{}: {}", c.id, c.name))
            .collect::<Vec<_>>()
            .join("\n")),
        "give_case" | "give-case" => {
            let mut query = args.trim();
            let mut count = 1;
            if let Some((q, n)) = query.rsplit_once(' ') {
                if let Ok(n) = n.parse::<u32>() {
                    query = q;
                    count = n;
                }
            }
            let case = catalog.find_case(query)?;
            inventory.transaction(path, |i| i.grant(case.id, count))?;
            Ok(format!(
                "Added {count} × {}. Open it in Inventory → Cases.",
                case.name
            ))
        }
        _ => Err("Commands: cases | give_case <name or ID> [count] | skins <search> | give_skin <weapon> <paint ID> [quality] | stickers <search> | give_sticker <ID> [count]".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qualities_stickers_and_shared_slots_survive_reload() {
        let mut i = Inventory::decode(
            "CSR-COSMETICS 1\nskin 1 weapon_ak47 801 27 0.05\nequip weapon_ak47 1\n",
        )
        .unwrap();
        i.skins[0].stattrak = Some(123456);
        i.skins[0].stickers[2] = Some(StickerSlot { kit: 10, wear: 0.4 });
        i.sticker_stock.insert(10, 3);
        i.skins.push(OwnedSkin {
            serial: 2,
            weapon: "weapon_knife_butterfly".into(),
            paint: 38,
            ..default()
        });
        i.next = 3;
        i.equip(2).unwrap();
        assert_eq!(i.active("weapon_knife_t").unwrap().serial, 2);
        assert_eq!(i.active("weapon_knife").unwrap().serial, 2);
        assert_eq!(Inventory::decode(&i.encode()).unwrap(), i);
        assert!(
            Inventory::decode("CSR-COSMETICS 2\nskin 1 sporty_gloves 10018 1 0.1\nquality 1 0 -\n")
                .is_err()
        );
        assert!(
            Inventory::decode("CSR-COSMETICS 2\nskin 1 weapon_ak47 801 1 0.1\nquality 1 0 20\n")
                .is_err()
        );
    }
    #[test]
    fn applying_consumes_one_sticker_and_scraping_never_refunds() {
        let mut c = Catalog::default();
        c.models.insert(
            "weapon_ak47".into(),
            ModelInfo {
                view: String::new(),
                world: String::new(),
                stickers: vec!["a".into(); 4],
            },
        );
        c.stickers.insert(
            10,
            StickerDefinition {
                name: "Test".into(),
                material: String::new(),
                event: 0,
                team: 0,
                player: 0,
            },
        );
        let mut i = Inventory::default();
        i.skins.push(OwnedSkin {
            serial: 1,
            weapon: "weapon_ak47".into(),
            ..default()
        });
        i.sticker_stock.insert(10, 2);
        i.apply_sticker(1, 2, 10, &c).unwrap();
        assert_eq!(i.sticker_stock[&10], 1);
        let before = i.clone();
        assert!(i.apply_sticker(1, 2, 10, &c).is_err());
        assert_eq!(i, before);
        for _ in 0..5 {
            i.scrape(1, 2).unwrap();
        }
        assert!(i.skins[0].stickers[2].is_none());
        assert_eq!(i.sticker_stock[&10], 1);
    }
    #[test]
    fn rare_and_souvenir_awards_respect_quality_rules() {
        let catalog = Catalog {
            finishes: vec![
                Finish {
                    weapon: "sporty_gloves".into(),
                    id: 10018,
                    rarity: 7,
                    wear: [0.06, 0.8],
                    ..default()
                },
                Finish {
                    weapon: "weapon_knife_butterfly".into(),
                    id: 38,
                    rarity: 7,
                    wear: [0., 0.08],
                    ..default()
                },
            ],
            ..default()
        };
        let mut rng = Random(42);
        for (contents, souvenir) in [(vec![0], false), (vec![1], false)] {
            let case = Case {
                id: 1,
                contents,
                souvenir,
                ..default()
            };
            let mut inventory = Inventory::default();
            inventory.grant(1, 100).unwrap();
            let mut tracked = 0;
            for _ in 0..100 {
                let item = inventory.open(&case, &catalog, &mut rng).unwrap();
                if is_glove(&item.weapon) {
                    assert_eq!(item.stattrak, None);
                } else if item.stattrak.is_some() {
                    tracked += 1;
                }
                assert_eq!(item.souvenir, None);
            }
            if case.contents[0] == 1 {
                assert!(tracked > 0 && tracked < 100);
            }
        }
    }
    fn temp() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "csr-cosmetics-{}-{}",
                std::process::id(),
                Random::fresh().0
            ))
            .join("inventory.cfg")
    }
    #[test]
    fn durable_transactions_merge_and_failed_mutations_do_not_commit() {
        let path = temp();
        let mut a = Inventory::default();
        let mut stale = Inventory::default();
        a.transaction(&path, |i| i.grant(4880, 2)).unwrap();
        stale.transaction(&path, |i| i.grant(4880, 3)).unwrap();
        assert_eq!(Inventory::load(&path).unwrap().cases[&4880], 5);
        assert!(a.transaction(&path, |i| i.grant(4880, 0)).is_err());
        assert_eq!(Inventory::load(&path).unwrap().cases[&4880], 5);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn invalid_profiles_are_never_silently_reset() {
        for s in [
            "bad",
            "CSR-COSMETICS 1\nskin 1 weapon_ak47 1 12 NaN",
            "CSR-COSMETICS 1\nequip weapon_ak47 99",
        ] {
            assert!(Inventory::decode(s).is_err());
        }
    }
    #[test]
    fn serials_conditions_and_equipment_round_trip() {
        let text =
            "CSR-COSMETICS 1\ncase 4880 3\nskin 8 weapon_ak47 801 999 0.06\nequip weapon_ak47 8\n";
        let i = Inventory::decode(text).unwrap();
        assert_eq!(i.next, 9);
        assert_eq!(i.active("weapon_ak47").unwrap().condition(), "Factory New");
        assert_eq!(Inventory::decode(&i.encode()).unwrap(), i);
    }
}
