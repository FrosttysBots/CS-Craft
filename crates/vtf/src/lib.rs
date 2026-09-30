//! Valve Texture Format reader (versions 7.0 to 7.5).
//!
//! A VTF holds a small thumbnail plus the full-resolution image with its
//! mip chain. Mip levels are stored smallest first; inside each level come
//! frames, then cube faces, then depth slices.
//!
//! [`Vtf::level`] returns raw bytes for one level so block-compressed
//! formats can go straight to the GPU; [`Vtf::rgba8`] decodes any supported
//! format on the CPU.

use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Format {
    Rgba8888 = 0,
    Abgr8888 = 1,
    Rgb888 = 2,
    Bgr888 = 3,
    Rgb565 = 4,
    I8 = 5,
    Ia88 = 6,
    P8 = 7,
    A8 = 8,
    Rgb888Bluescreen = 9,
    Bgr888Bluescreen = 10,
    Argb8888 = 11,
    Bgra8888 = 12,
    Dxt1 = 13,
    Dxt3 = 14,
    Dxt5 = 15,
    Bgrx8888 = 16,
    Bgr565 = 17,
    Bgrx5551 = 18,
    Bgra4444 = 19,
    Dxt1OneBitAlpha = 20,
    Bgra5551 = 21,
    Uv88 = 22,
    Uvwq8888 = 23,
    Rgba16161616F = 24,
    Rgba16161616 = 25,
    Uvlx8888 = 26,
}

impl Format {
    pub fn from_i32(v: i32) -> Option<Self> {
        use Format::*;
        Some(match v {
            0 => Rgba8888,
            1 => Abgr8888,
            2 => Rgb888,
            3 => Bgr888,
            4 => Rgb565,
            5 => I8,
            6 => Ia88,
            7 => P8,
            8 => A8,
            9 => Rgb888Bluescreen,
            10 => Bgr888Bluescreen,
            11 => Argb8888,
            12 => Bgra8888,
            13 => Dxt1,
            14 => Dxt3,
            15 => Dxt5,
            16 => Bgrx8888,
            17 => Bgr565,
            18 => Bgrx5551,
            19 => Bgra4444,
            20 => Dxt1OneBitAlpha,
            21 => Bgra5551,
            22 => Uv88,
            23 => Uvwq8888,
            24 => Rgba16161616F,
            25 => Rgba16161616,
            26 => Uvlx8888,
            _ => return None,
        })
    }

    /// Bytes per 4x4 block for block-compressed formats.
    pub fn block_bytes(self) -> Option<usize> {
        match self {
            Format::Dxt1 | Format::Dxt1OneBitAlpha => Some(8),
            Format::Dxt3 | Format::Dxt5 => Some(16),
            _ => None,
        }
    }

    /// Bytes per pixel for uncompressed formats.
    pub fn pixel_bytes(self) -> Option<usize> {
        use Format::*;
        match self {
            Rgba8888 | Abgr8888 | Argb8888 | Bgra8888 | Bgrx8888 | Uvwq8888 | Uvlx8888 => Some(4),
            Rgb888 | Bgr888 | Rgb888Bluescreen | Bgr888Bluescreen => Some(3),
            Rgb565 | Bgr565 | Bgrx5551 | Bgra4444 | Bgra5551 | Ia88 | Uv88 => Some(2),
            I8 | P8 | A8 => Some(1),
            Rgba16161616F | Rgba16161616 => Some(8),
            Dxt1 | Dxt1OneBitAlpha | Dxt3 | Dxt5 => None,
        }
    }

    pub fn has_alpha(self) -> bool {
        use Format::*;
        matches!(
            self,
            Rgba8888
                | Abgr8888
                | Argb8888
                | Bgra8888
                | Dxt3
                | Dxt5
                | Dxt1OneBitAlpha
                | Bgra4444
                | Bgra5551
                | Ia88
                | A8
                | Rgba16161616F
                | Rgba16161616
        )
    }

    /// Size in bytes of one image of `w x h x d` in this format.
    pub fn image_size(self, w: usize, h: usize, d: usize) -> usize {
        let (w, h, d) = (w.max(1), h.max(1), d.max(1));
        if let Some(b) = self.block_bytes() {
            w.div_ceil(4) * h.div_ceil(4) * b * d
        } else {
            w * h * d * self.pixel_bytes().unwrap_or(4)
        }
    }
}

/// VTF header flags we care about.
pub mod flags {
    pub const POINTSAMPLE: u32 = 0x0001;
    pub const TRILINEAR: u32 = 0x0002;
    pub const CLAMPS: u32 = 0x0004;
    pub const CLAMPT: u32 = 0x0008;
    pub const ANISOTROPIC: u32 = 0x0010;
    pub const NORMAL: u32 = 0x0080;
    pub const NOMIP: u32 = 0x0100;
    pub const ONEBITALPHA: u32 = 0x1000;
    pub const EIGHTBITALPHA: u32 = 0x2000;
    pub const ENVMAP: u32 = 0x4000;
    pub const SRGB: u32 = 0x0040;
}

#[derive(Clone, Debug)]
pub struct Vtf {
    pub version: (u32, u32),
    pub width: usize,
    pub height: usize,
    pub depth: usize,
    pub frames: usize,
    pub faces: usize,
    pub mip_count: usize,
    pub flags: u32,
    pub format: Format,
    pub reflectivity: [f32; 3],
    /// High-resolution image data, all mips (smallest first).
    data: Vec<u8>,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}

fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

impl Vtf {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 64 || &bytes[0..4] != b"VTF\0" {
            return Err(invalid("not a VTF file"));
        }
        let major = u32_at(bytes, 4);
        let minor = u32_at(bytes, 8);
        if major != 7 || minor > 5 {
            return Err(invalid(format!("unsupported VTF version {major}.{minor}")));
        }
        let header_size = u32_at(bytes, 12) as usize;
        let width = u16_at(bytes, 16) as usize;
        let height = u16_at(bytes, 18) as usize;
        let flags = u32_at(bytes, 20);
        let frames = (u16_at(bytes, 24) as usize).max(1);
        let first_frame = u16_at(bytes, 26);
        let reflectivity = [
            f32::from_bits(u32_at(bytes, 32)),
            f32::from_bits(u32_at(bytes, 36)),
            f32::from_bits(u32_at(bytes, 40)),
        ];
        let hi_format = u32_at(bytes, 52) as i32;
        let mip_count = (bytes[56] as usize).max(1);
        let lo_format = u32_at(bytes, 57) as i32;
        let lo_w = bytes[61] as usize;
        let lo_h = bytes[62] as usize;
        let depth = if minor >= 2 {
            (u16_at(bytes, 63) as usize).max(1)
        } else {
            1
        };
        let format = Format::from_i32(hi_format)
            .ok_or_else(|| invalid(format!("unknown VTF image format {hi_format}")))?;

        let faces = if flags & flags::ENVMAP != 0 {
            if minor < 5 && first_frame == 0xFFFF { 7 } else { 6 }
        } else {
            1
        };

        let mut total = 0usize;
        for mip in 0..mip_count {
            let (w, h, d) = mip_dims(width, height, depth, mip);
            total += format.image_size(w, h, d) * frames * faces;
        }

        let hi_offset = if minor >= 3 {
            let num_resources = u32_at(bytes, 68) as usize;
            let mut found = None;
            for i in 0..num_resources {
                let o = 80 + i * 8;
                if o + 8 > bytes.len() {
                    break;
                }
                if bytes[o..o + 3] == [0x30, 0, 0] {
                    found = Some(u32_at(bytes, o + 4) as usize);
                }
            }
            found.ok_or_else(|| invalid("VTF has no high-res image resource"))?
        } else {
            let lo_size = if lo_w > 0 && lo_h > 0 && lo_format >= 0 {
                Format::from_i32(lo_format).map_or(0, |f| f.image_size(lo_w, lo_h, 1))
            } else {
                0
            };
            header_size + lo_size
        };
        let data = bytes
            .get(hi_offset..hi_offset + total)
            .ok_or_else(|| {
                invalid(format!(
                    "VTF image data truncated (need {total} bytes at {hi_offset}, file is {})",
                    bytes.len()
                ))
            })?
            .to_vec();

        Ok(Self {
            version: (major, minor),
            width,
            height,
            depth,
            frames,
            faces,
            mip_count,
            flags,
            format,
            reflectivity,
            data,
        })
    }

    /// Dimensions of mip `level` (0 = full size).
    pub fn level_dims(&self, level: usize) -> (usize, usize) {
        let (w, h, _) = mip_dims(self.width, self.height, self.depth, level);
        (w, h)
    }

    /// Raw bytes of one image: mip `level` (0 = largest), frame, face, all slices.
    pub fn level(&self, level: usize, frame: usize, face: usize) -> Option<&[u8]> {
        if level >= self.mip_count || frame >= self.frames || face >= self.faces {
            return None;
        }
        // Stored smallest mip first: skip every level smaller than `level`.
        let mut offset = 0usize;
        for m in (level + 1..self.mip_count).rev() {
            let (w, h, d) = mip_dims(self.width, self.height, self.depth, m);
            offset += self.format.image_size(w, h, d) * self.frames * self.faces;
        }
        let (w, h, d) = mip_dims(self.width, self.height, self.depth, level);
        let image = self.format.image_size(w, h, d);
        offset += (frame * self.faces + face) * image;
        self.data.get(offset..offset + image)
    }

    /// Decodes one image of mip `level` (first slice) to RGBA8.
    pub fn rgba8(&self, level: usize, frame: usize, face: usize) -> Option<Vec<u8>> {
        let (w, h) = self.level_dims(level);
        let raw = self.level(level, frame, face)?;
        decode_rgba8(self.format, raw, w, h)
    }

    /// Decodes an HDR (`RGBA16161616F`) image to linear float RGBA.
    pub fn rgba_f32(&self, level: usize, frame: usize, face: usize) -> Option<Vec<[f32; 4]>> {
        let (w, h) = self.level_dims(level);
        let raw = self.level(level, frame, face)?;
        match self.format {
            Format::Rgba16161616F => Some(
                raw.chunks_exact(8)
                    .take(w * h)
                    .map(|c| {
                        [
                            f16_to_f32(u16::from_le_bytes([c[0], c[1]])),
                            f16_to_f32(u16::from_le_bytes([c[2], c[3]])),
                            f16_to_f32(u16::from_le_bytes([c[4], c[5]])),
                            f16_to_f32(u16::from_le_bytes([c[6], c[7]])),
                        ]
                    })
                    .collect(),
            ),
            _ => decode_rgba8(self.format, raw, w, h).map(|p| {
                p.chunks_exact(4)
                    .map(|c| {
                        [
                            c[0] as f32 / 255.0,
                            c[1] as f32 / 255.0,
                            c[2] as f32 / 255.0,
                            c[3] as f32 / 255.0,
                        ]
                    })
                    .collect()
            }),
        }
    }
}

fn mip_dims(w: usize, h: usize, d: usize, level: usize) -> (usize, usize, usize) {
    ((w >> level).max(1), (h >> level).max(1), (d >> level).max(1))
}

fn expand565(v: u16) -> (u8, u8, u8) {
    let r = ((v >> 11) & 0x1F) as u8;
    let g = ((v >> 5) & 0x3F) as u8;
    let b = (v & 0x1F) as u8;
    ((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2))
}

/// Decodes one image to tightly packed RGBA8.
pub fn decode_rgba8(format: Format, raw: &[u8], w: usize, h: usize) -> Option<Vec<u8>> {
    let px = w * h;
    let mut out = vec![0u8; px * 4];
    if let Some(block) = format.block_bytes() {
        let bw = w.div_ceil(4);
        let bh = h.div_ceil(4);
        let mut tmp = [0u8; 64];
        for by in 0..bh {
            for bx in 0..bw {
                let o = (by * bw + bx) * block;
                let src = raw.get(o..o + block)?;
                match format {
                    Format::Dxt1 | Format::Dxt1OneBitAlpha => bcdec_rs::bc1(src, &mut tmp, 16),
                    Format::Dxt3 => bcdec_rs::bc2(src, &mut tmp, 16),
                    Format::Dxt5 => bcdec_rs::bc3(src, &mut tmp, 16),
                    _ => unreachable!(),
                }
                for y in 0..4 {
                    let py = by * 4 + y;
                    if py >= h {
                        break;
                    }
                    for x in 0..4 {
                        let pxx = bx * 4 + x;
                        if pxx >= w {
                            break;
                        }
                        let d = (py * w + pxx) * 4;
                        let s = y * 16 + x * 4;
                        out[d..d + 4].copy_from_slice(&tmp[s..s + 4]);
                    }
                }
            }
        }
        return Some(out);
    }
    let bpp = format.pixel_bytes()?;
    if raw.len() < px * bpp {
        return None;
    }
    for i in 0..px {
        let s = &raw[i * bpp..i * bpp + bpp];
        let rgba: [u8; 4] = match format {
            Format::Rgba8888 | Format::Uvwq8888 | Format::Uvlx8888 => [s[0], s[1], s[2], s[3]],
            Format::Abgr8888 => [s[3], s[2], s[1], s[0]],
            Format::Argb8888 => [s[1], s[2], s[3], s[0]],
            Format::Bgra8888 => [s[2], s[1], s[0], s[3]],
            Format::Bgrx8888 => [s[2], s[1], s[0], 255],
            Format::Rgb888 => [s[0], s[1], s[2], 255],
            Format::Bgr888 => [s[2], s[1], s[0], 255],
            Format::Rgb888Bluescreen => {
                if s == [0, 0, 255] { [0, 0, 0, 0] } else { [s[0], s[1], s[2], 255] }
            }
            Format::Bgr888Bluescreen => {
                if s == [255, 0, 0] { [0, 0, 0, 0] } else { [s[2], s[1], s[0], 255] }
            }
            Format::Rgb565 => {
                let (r, g, b) = expand565(u16::from_le_bytes([s[0], s[1]]));
                [r, g, b, 255]
            }
            Format::Bgr565 => {
                let (b, g, r) = expand565(u16::from_le_bytes([s[0], s[1]]));
                [r, g, b, 255]
            }
            Format::Bgra4444 => {
                let v = u16::from_le_bytes([s[0], s[1]]);
                let n = |x: u16| ((x & 0xF) as u8) * 17;
                [n(v >> 8), n(v >> 4), n(v), n(v >> 12)]
            }
            Format::Bgrx5551 | Format::Bgra5551 => {
                let v = u16::from_le_bytes([s[0], s[1]]);
                let n = |x: u16| {
                    let x = (x & 0x1F) as u8;
                    (x << 3) | (x >> 2)
                };
                let a = if format == Format::Bgra5551 && v & 0x8000 == 0 { 0 } else { 255 };
                [n(v >> 10), n(v >> 5), n(v), a]
            }
            Format::I8 => [s[0], s[0], s[0], 255],
            Format::P8 => [s[0], s[0], s[0], 255],
            Format::A8 => [0, 0, 0, s[0]],
            Format::Ia88 => [s[0], s[0], s[0], s[1]],
            Format::Uv88 => [s[0], s[1], 0, 255],
            Format::Rgba16161616F => {
                let c = |o: usize| {
                    let f = f16_to_f32(u16::from_le_bytes([s[o], s[o + 1]]));
                    (f.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
                };
                [c(0), c(2), c(4), c(6)]
            }
            Format::Rgba16161616 => {
                let c = |o: usize| (u16::from_le_bytes([s[o], s[o + 1]]) >> 8) as u8;
                [c(0), c(2), c(4), c(6)]
            }
            Format::Dxt1 | Format::Dxt1OneBitAlpha | Format::Dxt3 | Format::Dxt5 => unreachable!(),
        };
        out[i * 4..i * 4 + 4].copy_from_slice(&rgba);
    }
    Some(out)
}

/// IEEE half → f32.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1F) as u32;
    let mant = (h & 0x3FF) as u32;
    let bits = if exp == 0 {
        if mant == 0 {
            sign << 31
        } else {
            // Subnormal: normalise.
            let mut e = 127 - 15 + 1;
            let mut m = mant;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            (sign << 31) | ((e as u32) << 23) | ((m & 0x3FF) << 13)
        }
    } else if exp == 31 {
        (sign << 31) | (0xFF << 23) | (mant << 13)
    } else {
        (sign << 31) | ((exp + 127 - 15) << 23) | (mant << 13)
    };
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(format: i32, w: u16, h: u16, mips: u8, minor: u32) -> Vec<u8> {
        let mut b = vec![0u8; 80];
        b[0..4].copy_from_slice(b"VTF\0");
        b[4..8].copy_from_slice(&7u32.to_le_bytes());
        b[8..12].copy_from_slice(&minor.to_le_bytes());
        b[12..16].copy_from_slice(&80u32.to_le_bytes());
        b[16..18].copy_from_slice(&w.to_le_bytes());
        b[18..20].copy_from_slice(&h.to_le_bytes());
        b[24..26].copy_from_slice(&1u16.to_le_bytes());
        b[52..56].copy_from_slice(&format.to_le_bytes());
        b[56] = mips;
        b[57..61].copy_from_slice(&(-1i32).to_le_bytes());
        b[63..65].copy_from_slice(&1u16.to_le_bytes());
        b
    }

    #[test]
    fn bgr888_mips_smallest_first() {
        // 2x2 with a 1x1 mip. File order: 1x1 then 2x2.
        let mut b = header(Format::Bgr888 as i32, 2, 2, 2, 2);
        b.extend_from_slice(&[1, 2, 3]); // 1x1 mip
        b.extend_from_slice(&[10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120]);
        let v = Vtf::parse(&b).unwrap();
        assert_eq!((v.width, v.height, v.mip_count), (2, 2, 2));
        let top = v.rgba8(0, 0, 0).unwrap();
        assert_eq!(&top[0..4], &[30, 20, 10, 255]);
        let small = v.rgba8(1, 0, 0).unwrap();
        assert_eq!(small, vec![3, 2, 1, 255]);
    }

    #[test]
    fn dxt1_solid_block() {
        let mut b = header(Format::Dxt1 as i32, 4, 4, 1, 2);
        // color0 = color1 = pure red (565: 0xF800), all indices 0.
        b.extend_from_slice(&[0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0]);
        let v = Vtf::parse(&b).unwrap();
        let px = v.rgba8(0, 0, 0).unwrap();
        assert_eq!(&px[0..4], &[255, 0, 0, 255]);
        assert_eq!(px.len(), 64);
    }

    #[test]
    fn half_floats() {
        assert_eq!(f16_to_f32(0x3C00), 1.0);
        assert_eq!(f16_to_f32(0xC000), -2.0);
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert!((f16_to_f32(0x3555) - 0.333).abs() < 0.001);
    }
}
