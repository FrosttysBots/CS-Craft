//! Content identity shared across games.
//!
//! Every piece of content (weapon, map, model, sound...) gets a stable,
//! human-readable id of the form `<origin>_<kind>_<name>`:
//!
//! * `csgo_weapon_ak47`
//! * `csgo_map_de_dust2`
//! * `iw4_weapon_m4a1` (MW2 content in the COD Legacy Mashup)
//!
//! The scheme matches the COD Legacy Mashup project so that loadouts, maps
//! and assets from several games can live in one catalog: the origin is
//! metadata, not a wall.

use std::fmt;
use std::str::FromStr;

/// The game a piece of content came from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GameOrigin {
    /// Counter-Strike: Global Offensive (Source).
    Csgo,
    /// Call of Duty: World at War.
    T4,
    /// Call of Duty: Modern Warfare 2.
    Iw4,
    /// Call of Duty: Black Ops.
    T5,
    /// Call of Duty: Modern Warfare 3.
    Iw5,
    /// Call of Duty: Black Ops II.
    T6,
    /// User or community content.
    Mod,
    /// Any other game, by prefix.
    Other(String),
}

impl GameOrigin {
    pub fn prefix(&self) -> &str {
        match self {
            GameOrigin::Csgo => "csgo",
            GameOrigin::T4 => "t4",
            GameOrigin::Iw4 => "iw4",
            GameOrigin::T5 => "t5",
            GameOrigin::Iw5 => "iw5",
            GameOrigin::T6 => "t6",
            GameOrigin::Mod => "mod",
            GameOrigin::Other(p) => p,
        }
    }

    pub fn from_prefix(prefix: &str) -> GameOrigin {
        match prefix {
            "csgo" => GameOrigin::Csgo,
            "t4" => GameOrigin::T4,
            "iw4" => GameOrigin::Iw4,
            "t5" => GameOrigin::T5,
            "iw5" => GameOrigin::Iw5,
            "t6" => GameOrigin::T6,
            "mod" => GameOrigin::Mod,
            other => GameOrigin::Other(other.to_string()),
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            GameOrigin::Csgo => "Counter-Strike: Global Offensive",
            GameOrigin::T4 => "World at War",
            GameOrigin::Iw4 => "Modern Warfare 2",
            GameOrigin::T5 => "Black Ops",
            GameOrigin::Iw5 => "Modern Warfare 3",
            GameOrigin::T6 => "Black Ops II",
            GameOrigin::Mod => "Mod",
            GameOrigin::Other(p) => p,
        }
    }
}

/// Content categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContentKind {
    Weapon,
    Map,
    Model,
    Material,
    Sound,
    Perk,
    Streak,
    Equipment,
    Character,
    Mode,
}

impl ContentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ContentKind::Weapon => "weapon",
            ContentKind::Map => "map",
            ContentKind::Model => "model",
            ContentKind::Material => "material",
            ContentKind::Sound => "sound",
            ContentKind::Perk => "perk",
            ContentKind::Streak => "streak",
            ContentKind::Equipment => "equipment",
            ContentKind::Character => "character",
            ContentKind::Mode => "mode",
        }
    }

    pub fn parse(s: &str) -> Option<ContentKind> {
        Some(match s {
            "weapon" => ContentKind::Weapon,
            "map" => ContentKind::Map,
            "model" => ContentKind::Model,
            "material" => ContentKind::Material,
            "sound" => ContentKind::Sound,
            "perk" => ContentKind::Perk,
            "streak" => ContentKind::Streak,
            "equipment" => ContentKind::Equipment,
            "character" => ContentKind::Character,
            "mode" => ContentKind::Mode,
            _ => return None,
        })
    }
}

/// `<origin>_<kind>_<name>`, e.g. `csgo_weapon_m4a1_silencer`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentId {
    pub origin: GameOrigin,
    pub kind: ContentKind,
    /// Lowercase `[a-z0-9_]`, may contain underscores.
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseIdError(pub String);

impl fmt::Display for ParseIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid content id: {}", self.0)
    }
}

impl std::error::Error for ParseIdError {}

/// Normalises a free-form name into id-safe form: lowercase ASCII
/// alphanumerics with single underscores.
pub fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_us = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_us = false;
        } else if !last_us {
            out.push('_');
            last_us = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

impl ContentId {
    pub fn new(origin: GameOrigin, kind: ContentKind, name: &str) -> Self {
        Self {
            origin,
            kind,
            name: sanitize(name),
        }
    }

    pub fn csgo(kind: ContentKind, name: &str) -> Self {
        Self::new(GameOrigin::Csgo, kind, name)
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}_{}", self.origin.prefix(), self.kind.as_str(), self.name)
    }
}

impl FromStr for ContentId {
    type Err = ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseIdError(s.to_string());
        let (origin, rest) = s.split_once('_').ok_or_else(err)?;
        let (kind, name) = rest.split_once('_').ok_or_else(err)?;
        let kind = ContentKind::parse(kind).ok_or_else(err)?;
        if origin.is_empty() || name.is_empty() || sanitize(name) != name || sanitize(origin) != origin {
            return Err(err());
        }
        Ok(ContentId {
            origin: GameOrigin::from_prefix(origin),
            kind,
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let id = ContentId::csgo(ContentKind::Weapon, "weapon_m4a1_silencer");
        assert_eq!(id.to_string(), "csgo_weapon_weapon_m4a1_silencer");
        let id = ContentId::csgo(ContentKind::Weapon, "M4A1-S");
        assert_eq!(id.to_string(), "csgo_weapon_m4a1_s");
        let parsed: ContentId = "csgo_weapon_m4a1_s".parse().unwrap();
        assert_eq!(parsed, id);
        let mw2: ContentId = "iw4_weapon_m4a1".parse().unwrap();
        assert_eq!(mw2.origin, GameOrigin::Iw4);
    }

    #[test]
    fn rejects_bad_ids() {
        assert!("csgo".parse::<ContentId>().is_err());
        assert!("csgo_gun_ak47".parse::<ContentId>().is_err());
        assert!("csgo_weapon_AK47".parse::<ContentId>().is_err());
    }

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize("  Desert Eagle!! "), "desert_eagle");
        assert_eq!(sanitize("de_dust2"), "de_dust2");
    }
}
