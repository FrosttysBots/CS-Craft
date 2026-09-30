//! Static props from the game lump (`sprp`).
//!
//! The game lump is a directory of sub-lumps whose offsets are absolute file
//! offsets. `sprp` holds a model-name dictionary, a leaf list, and one fixed
//! record per prop. The record size grows with the lump version, so the
//! stride is derived from the remaining byte count and only the leading
//! fields shared by every version are decoded (plus uniform scale when the
//! record is large enough to carry it).

use crate::lump::{self, Rec};
use glam::Vec3;
use std::io;

const SPRP: i32 = i32::from_be_bytes(*b"sprp");

#[derive(Clone, Debug)]
pub struct StaticProp {
    pub origin: Vec3,
    /// Pitch, yaw, roll in degrees (Source `QAngle`).
    pub angles: Vec3,
    pub model: u16,
    pub solid: u8,
    pub flags: u8,
    pub skin: i32,
    pub fade_min: f32,
    pub fade_max: f32,
    pub scale: f32,
}

#[derive(Clone, Debug, Default)]
pub struct StaticProps {
    pub version: u16,
    pub models: Vec<String>,
    pub props: Vec<StaticProp>,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

pub fn parse(file: &[u8], game_lump: &[u8]) -> io::Result<StaticProps> {
    if game_lump.len() < 4 {
        return Ok(StaticProps::default());
    }
    let count = i32::from_le_bytes(game_lump[0..4].try_into().unwrap()).max(0) as usize;
    for i in 0..count {
        let o = 4 + i * 16;
        let Some(entry) = game_lump.get(o..o + 16) else { break };
        let r = Rec(entry);
        if r.i32(0) != SPRP {
            continue;
        }
        let flags = r.u16(4);
        let version = r.u16(6);
        let ofs = r.i32(8).max(0) as usize;
        let len = r.i32(12).max(0) as usize;
        let raw = file
            .get(ofs..)
            .ok_or_else(|| invalid("sprp offset past end of file"))?;
        let data = if flags & 1 != 0 || raw.starts_with(b"LZMA") {
            lump::decompress_valve_lzma(raw)?
        } else {
            raw.get(..len)
                .ok_or_else(|| invalid("sprp runs past end of file"))?
                .to_vec()
        };
        return parse_sprp(&data, version);
    }
    Ok(StaticProps::default())
}

fn parse_sprp(d: &[u8], version: u16) -> io::Result<StaticProps> {
    let mut p = 0usize;
    let read_i32 = |p: &mut usize| -> io::Result<usize> {
        let b = d.get(*p..*p + 4).ok_or_else(|| invalid("truncated sprp"))?;
        *p += 4;
        Ok(i32::from_le_bytes(b.try_into().unwrap()).max(0) as usize)
    };
    let dict = read_i32(&mut p)?;
    let mut models = Vec::with_capacity(dict);
    for _ in 0..dict {
        let name = d.get(p..p + 128).ok_or_else(|| invalid("truncated sprp dict"))?;
        let end = name.iter().position(|&b| b == 0).unwrap_or(128);
        models.push(String::from_utf8_lossy(&name[..end]).into_owned());
        p += 128;
    }
    let leaves = read_i32(&mut p)?;
    p += leaves * 2;
    let n = read_i32(&mut p)?;
    let mut props = Vec::with_capacity(n);
    if n > 0 {
        let rest = d.get(p..).ok_or_else(|| invalid("truncated sprp props"))?;
        let stride = rest.len() / n;
        if stride < 60 {
            return Err(invalid(format!("sprp record too small ({stride} bytes)")));
        }
        for chunk in rest.chunks_exact(stride).take(n) {
            let r = Rec(chunk);
            props.push(StaticProp {
                origin: r.vec3(0),
                angles: r.vec3(12),
                model: r.u16(24),
                solid: r.u8(30),
                flags: r.u8(31),
                skin: r.i32(32),
                fade_min: r.f32(36),
                fade_max: r.f32(40),
                scale: if version >= 11 && stride >= 80 { r.f32(76) } else { 1.0 },
            });
        }
    }
    Ok(StaticProps {
        version,
        models,
        props,
    })
}
