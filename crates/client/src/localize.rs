//! Display strings from the game's `resource/csgo_english.txt`.
//!
//! The file is UTF-16 LE KeyValues: `"lang" { "Tokens" { "SFUI_WPNHUD_AK47"
//! "AK-47" ... } }`. Items name their token (`#SFUI_WPNHUD_AK47`), which is
//! looked up here without the `#`.

use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Resource, Default)]
pub struct Localization {
    tokens: HashMap<String, String>,
}

fn decode_utf16(bytes: &[u8]) -> Option<String> {
    let body = bytes.strip_prefix(&[0xFF, 0xFE])?;
    let units: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&units))
}

impl Localization {
    pub fn load(fs: &vfs::Vfs) -> Self {
        let Some(text) = fs.read("resource/csgo_english.txt").and_then(|b| decode_utf16(&b)) else {
            return Self::default();
        };
        let mut tokens = HashMap::new();
        if let Ok(doc) = kv::parse_escaped(&text) {
            if let Some(t) = doc.block("lang").and_then(|l| l.block("Tokens")) {
                for (k, v) in t.iter() {
                    if let Some(s) = v.as_str() {
                        tokens.insert(k.to_ascii_lowercase(), s.to_string());
                    }
                }
            }
        }
        Self { tokens }
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Resolves `#TOKEN` (or `TOKEN`); `None` if unknown.
    pub fn get(&self, token: &str) -> Option<&str> {
        self.tokens.get(&token.trim_start_matches('#').to_ascii_lowercase()).map(String::as_str)
    }

    /// A weapon's display name, falling back to its class name.
    pub fn weapon_name(&self, def: &weapons::WeaponDefinition) -> String {
        self.get(&def.name_token)
            .map(str::to_string)
            .unwrap_or_else(|| def.class_name.trim_start_matches("weapon_").to_uppercase())
    }
}
