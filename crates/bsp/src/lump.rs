//! BSP header, lump access and little-endian record decoding.

use glam::{Vec3, Vec4};
use std::io;

pub const HEADER_LUMPS: usize = 64;
const IDENT: &[u8; 4] = b"VBSP";
const LZMA_ID: &[u8; 4] = b"LZMA";

#[derive(Clone, Copy, Debug, Default)]
pub struct LumpInfo {
    pub offset: i32,
    pub length: i32,
    pub version: i32,
    /// Uncompressed size when the lump is LZMA compressed, else 0.
    pub four_cc: i32,
}

#[derive(Clone, Debug)]
pub struct Header {
    pub version: i32,
    pub lumps: [LumpInfo; HEADER_LUMPS],
    pub map_revision: i32,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn i32_at(d: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

impl Header {
    pub fn parse(file: &[u8]) -> io::Result<Self> {
        let header_len = 8 + HEADER_LUMPS * 16 + 4;
        if file.len() < header_len {
            return Err(invalid("file too small for a BSP header"));
        }
        if &file[0..4] != IDENT {
            return Err(invalid("not a VBSP file"));
        }
        let version = i32_at(file, 4);
        let mut lumps = [LumpInfo::default(); HEADER_LUMPS];
        for (i, l) in lumps.iter_mut().enumerate() {
            let o = 8 + i * 16;
            *l = LumpInfo {
                offset: i32_at(file, o),
                length: i32_at(file, o + 4),
                version: i32_at(file, o + 8),
                four_cc: i32_at(file, o + 12),
            };
        }
        let map_revision = i32_at(file, 8 + HEADER_LUMPS * 16);
        Ok(Self {
            version,
            lumps,
            map_revision,
        })
    }

    /// Returns a lump's bytes, decompressing LZMA lumps.
    pub fn lump(&self, file: &[u8], index: usize) -> io::Result<Vec<u8>> {
        let l = self.lumps[index];
        if l.length <= 0 {
            return Ok(Vec::new());
        }
        let start = l.offset as usize;
        let end = start + l.length as usize;
        let raw = file
            .get(start..end)
            .ok_or_else(|| invalid(format!("lump {index} runs past end of file")))?;
        // The pakfile is never compressed as a whole (its entries may be).
        if index != crate::lumps::PAKFILE && raw.len() >= 17 && &raw[0..4] == LZMA_ID {
            return decompress_valve_lzma(raw);
        }
        Ok(raw.to_vec())
    }
}

/// Decodes Valve's LZMA wrapper: `"LZMA"`, u32 actual size, u32 lzma size,
/// 5 property bytes, then the raw stream.
pub fn decompress_valve_lzma(raw: &[u8]) -> io::Result<Vec<u8>> {
    if raw.len() < 17 || &raw[0..4] != LZMA_ID {
        return Err(invalid("missing LZMA header"));
    }
    let actual = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as u64;
    let lzma_size = u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]) as usize;
    let end = (17 + lzma_size).min(raw.len());
    let mut input = io::BufReader::new(&raw[12..end]);
    let mut out = Vec::with_capacity(actual as usize);
    let opts = lzma_rs::decompress::Options {
        unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(actual)),
        ..Default::default()
    };
    lzma_rs::lzma_decompress_with_options(&mut input, &mut out, &opts)
        .map_err(|e| invalid(format!("lzma: {e:?}")))?;
    Ok(out)
}

/// A fixed-size little-endian record.
pub struct Rec<'a>(pub &'a [u8]);

impl Rec<'_> {
    pub fn u8(&self, o: usize) -> u8 {
        self.0[o]
    }
    pub fn u16(&self, o: usize) -> u16 {
        u16::from_le_bytes([self.0[o], self.0[o + 1]])
    }
    pub fn i16(&self, o: usize) -> i16 {
        i16::from_le_bytes([self.0[o], self.0[o + 1]])
    }
    pub fn i32(&self, o: usize) -> i32 {
        i32_at(self.0, o)
    }
    pub fn u32(&self, o: usize) -> u32 {
        self.i32(o) as u32
    }
    pub fn f32(&self, o: usize) -> f32 {
        f32::from_bits(self.u32(o))
    }
    pub fn vec3(&self, o: usize) -> Vec3 {
        Vec3::new(self.f32(o), self.f32(o + 4), self.f32(o + 8))
    }
    pub fn vec4(&self, o: usize) -> Vec4 {
        Vec4::new(self.f32(o), self.f32(o + 4), self.f32(o + 8), self.f32(o + 12))
    }
}

/// Decodes `bytes.len() / stride` records. A trailing partial record is ignored.
pub fn records<T>(bytes: &[u8], stride: usize, f: impl Fn(Rec) -> T) -> Vec<T> {
    bytes.chunks_exact(stride).map(|c| f(Rec(c))).collect()
}
