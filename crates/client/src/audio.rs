//! Game sounds from CS:GO soundscripts.
//!
//! `scripts/game_sounds_manifest.txt` lists soundscript files; each maps an
//! entry name (`Weapon_AK47.Single`) to one `wave` or several `rndwave`
//! files under `sound/`. Wave paths may start with flag characters (`)`,
//! `^`, `*`, ...) that select mixing behaviour in the original engine; they
//! are stripped here.

use crate::coords::to_bevy;
use bevy::audio::{AudioPlayer, AudioSource, PlaybackSettings, SpatialScale, Volume};
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

/// Source units per metre for spatial attenuation (1 m ≈ 39.37 in), with a
/// little extra reach so gunfire carries across a map as it does in-game.
const UNITS_PER_METER: f32 = 39.37 * 4.0;

#[derive(Clone, Debug, Default)]
struct Entry {
    waves: Vec<String>,
    volume: f32,
}

#[derive(Component)]
pub struct SoundGain(pub f32);

#[derive(Resource, Default)]
pub struct SoundLibrary {
    entries: HashMap<String, Entry>,
    loaded: HashMap<String, Option<Handle<AudioSource>>>,
    counter: u64,
    pub master_volume: f32,
}

/// Returns bytes Bevy's decoder is guaranteed to accept, or `None`.
///
/// Bevy panics on undecodable audio, so every file is checked first:
/// PCM (8/16/24/32-bit int, 32-bit float) WAVs pass through, MP3-in-WAV
/// (format 0x55, common in Valve content) is unwrapped to its raw MP3
/// frames, bare MP3s pass through, anything else is skipped.
pub fn playable(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.starts_with(b"ID3") || (bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0) {
        return Some(bytes);
    }
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let u16_at = |o: usize| bytes.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| bytes.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let mut format = None;
    let mut data = None;
    let mut p = 12;
    while p + 8 <= bytes.len() {
        let id = &bytes[p..p + 4];
        let size = u32_at(p + 4)? as usize;
        let body = p + 8;
        match id {
            b"fmt " => {
                let mut tag = u16_at(body)?;
                let bits = u16_at(body + 14)?;
                // WAVE_FORMAT_EXTENSIBLE: the real tag is the sub-format GUID's first word.
                if tag == 0xFFFE {
                    tag = u16_at(body + 24)?;
                }
                format = Some((tag, bits));
            }
            b"data" => data = Some((body, size.min(bytes.len().saturating_sub(body)))),
            _ => {}
        }
        p = body + size + (size & 1);
    }
    match (format?, data?) {
        ((1, 8 | 16 | 24 | 32), _) | ((3, 32), _) => Some(bytes),
        ((0x55, _), (start, len)) => Some(bytes[start..start + len].to_vec()),
        _ => None,
    }
}

fn strip_flags(wave: &str) -> String {
    let w = wave.trim_start_matches(|c: char| "*#@><^)}$!?&~`+%(".contains(c));
    format!("sound/{}", w.replace('\\', "/").to_ascii_lowercase())
}

impl SoundLibrary {
    pub fn load(fs: &vfs::Vfs) -> Self {
        let mut lib = SoundLibrary {
            master_volume: 0.5,
            ..Default::default()
        };
        let mut files: Vec<String> = Vec::new();
        if let Some(manifest) = fs.read_string("scripts/game_sounds_manifest.txt").and_then(|t| kv::parse(&t).ok()) {
            for (_, v) in manifest.iter() {
                if let Some(b) = v.as_block() {
                    for (k, f) in b.iter() {
                        if k.to_ascii_lowercase().starts_with("precache_file") || k.eq_ignore_ascii_case("preload_file") {
                            if let Some(f) = f.as_str() {
                                files.push(f.to_string());
                            }
                        }
                    }
                }
            }
        }
        if files.is_empty() {
            files.push("scripts/game_sounds_weapons.txt".into());
        }
        for file in files {
            let Some(doc) = fs.read_string(&file).and_then(|t| kv::parse(&t).ok()) else { continue };
            for (name, v) in doc.iter() {
                let Some(b) = v.as_block() else { continue };
                let mut waves = Vec::new();
                if let Some(w) = b.str("wave") {
                    waves.push(strip_flags(w));
                }
                if let Some(r) = b.block("rndwave") {
                    for (_, w) in r.iter() {
                        if let Some(w) = w.as_str() {
                            waves.push(strip_flags(w));
                        }
                    }
                }
                if waves.is_empty() {
                    continue;
                }
                let volume = b.str("volume").and_then(kv::parse_f32).unwrap_or(1.0).clamp(0.0, 1.0);
                lib.entries.insert(name.to_ascii_lowercase(), Entry { waves, volume });
            }
        }
        lib
    }

    /// Soundscript entry names (lower case).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The files an entry picks from.
    pub fn waves(&self, name: &str) -> Option<&[String]> {
        self.entries.get(&name.to_ascii_lowercase()).map(|e| e.waves.as_slice())
    }

    pub fn has(&self, name: &str) -> bool {
        self.entries.contains_key(&name.to_ascii_lowercase())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn source(&mut self, fs: &vfs::Vfs, sources: &mut Assets<AudioSource>, path: &str) -> Option<Handle<AudioSource>> {
        self.loaded
            .entry(path.to_string())
            .or_insert_with(|| {
                let bytes = playable(fs.read(path)?)?;
                Some(sources.add(AudioSource { bytes: Arc::from(bytes) }))
            })
            .clone()
    }

    /// Plays a soundscript entry. `at` is a Source-space position for
    /// spatial sounds, `None` for sounds heard at the listener (own gun).
    pub fn play(
        &mut self,
        commands: &mut Commands,
        fs: &vfs::Vfs,
        sources: &mut Assets<AudioSource>,
        name: &str,
        at: Option<glam::Vec3>,
    ) {
        let Some(entry) = self.entries.get(&name.to_ascii_lowercase()).cloned() else { return };
        self.counter = self.counter.wrapping_add(1);
        let pick = (self.counter as usize * 7919) % entry.waves.len();
        let Some(handle) = self.source(fs, sources, &entry.waves[pick]) else { return };
        let volume = Volume::Linear(entry.volume * self.master_volume);
        match at {
            Some(pos) => {
                commands.spawn((
                    AudioPlayer(handle), SoundGain(entry.volume),
                    PlaybackSettings::DESPAWN
                        .with_volume(volume)
                        .with_spatial(true)
                        .with_spatial_scale(SpatialScale::new(1.0 / UNITS_PER_METER)),
                    Transform::from_translation(to_bevy(pos)),
                ));
            }
            None => {
                commands.spawn((AudioPlayer(handle), SoundGain(entry.volume), PlaybackSettings::DESPAWN.with_volume(volume)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(tag: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&tag.to_le_bytes());
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&44100u32.to_le_bytes());
        fmt.extend_from_slice(&88200u32.to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        out.extend_from_slice(&fmt);
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn accepts_pcm_unwraps_mp3_rejects_adpcm() {
        assert!(playable(wav(1, 16, &[0, 0, 1, 0])).is_some());
        assert_eq!(playable(wav(0x55, 0, &[0xFF, 0xFB, 1, 2])).unwrap(), vec![0xFF, 0xFB, 1, 2]);
        assert!(playable(wav(2, 4, &[0, 0])).is_none());
        assert!(playable(b"not audio".to_vec()).is_none());
    }
}
