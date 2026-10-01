//! Studio model animations and sequences.
//!
//! An animation (`mstudioanimdesc_t`) owns per-bone tracks. Each bone
//! record has a small header (bone, flags, offset to the next record)
//! followed by rotation then position data. Each part is either a single
//! raw value (`Quaternion48`/`Quaternion64`, `Vector48`) or three
//! run-length-encoded value streams, one per axis, scaled by the bone's
//! `rot_scale`/`pos_scale`. Long animations are split into sections; some
//! models keep animation data in a separate `.ani` file (anim blocks).
//!
//! A sequence names one or more animations (a blend grid) plus playback
//! metadata.

use crate::skeleton::{euler_to_quat, Bone};
use crate::{invalid, R};
use glam::{Quat, Vec3};
use std::io;

pub const ANIMDESC_STRIDE: usize = 100;
pub const SEQDESC_STRIDE: usize = 212;

/// `mstudioanimdesc_t` flags.
pub mod flags {
    pub const LOOPING: i32 = 0x0001;
    pub const DELTA: i32 = 0x0004;
    pub const ALLZEROS: i32 = 0x0020;
    pub const FRAMEANIM: i32 = 0x0040;
}

const ANIM_RAWPOS: u8 = 0x01;
const ANIM_RAWROT: u8 = 0x02;
const ANIM_ANIMPOS: u8 = 0x04;
const ANIM_ANIMROT: u8 = 0x08;
const ANIM_DELTA: u8 = 0x10;
const ANIM_RAWROT2: u8 = 0x20;

#[derive(Clone, Debug)]
pub struct AnimDesc {
    pub name: String,
    pub fps: f32,
    pub flags: i32,
    pub num_frames: usize,
    /// Anim block holding the data (0 = inside the .mdl).
    pub anim_block: i32,
    /// Offset of the first bone record (absolute in the .mdl, or relative
    /// to the block start for block > 0).
    pub anim_offset: usize,
    pub section_frames: usize,
    /// (block, absolute-or-block-relative offset) per section.
    pub sections: Vec<(i32, usize)>,
}

impl AnimDesc {
    pub fn duration(&self) -> f32 {
        if self.fps > 0.0 && self.num_frames > 1 {
            (self.num_frames - 1) as f32 / self.fps
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug)]
pub struct Sequence {
    pub name: String,
    pub activity: String,
    pub flags: i32,
    /// Animation indices, `group_size[0] * group_size[1]` (blend grid).
    pub anims: Vec<i16>,
    pub group_size: [i32; 2],
    pub param_index: [i32; 2],
    pub param_start: [f32; 2],
    pub param_end: [f32; 2],
    pub fade_in: f32,
    pub fade_out: f32,
    /// Per-bone weights (0 = the sequence leaves that bone alone, keeping
    /// whatever pose it already had). Empty means all ones.
    pub bone_weights: Vec<f32>,
}

impl Sequence {
    pub fn weight(&self, bone: usize) -> f32 {
        self.bone_weights.get(bone).copied().unwrap_or(1.0)
    }

    pub fn looping(&self) -> bool {
        self.flags & flags::LOOPING != 0
    }
}

pub(crate) fn read_anim_descs(r: &R, count: usize, index: usize) -> io::Result<Vec<AnimDesc>> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = index + i * ANIMDESC_STRIDE;
        let name_ofs = r.i32(o + 4)?;
        let name = r.cstr((o as i64 + name_ofs as i64) as usize)?;
        let anim_block = r.i32(o + 52)?;
        let anim_index = r.i32(o + 56)?;
        let section_index = r.i32(o + 80)?;
        let section_frames = r.i32(o + 84)?.max(0) as usize;
        let num_frames = r.i32(o + 16)?.max(0) as usize;
        let rel = |block: i32, idx: i32| -> usize {
            if block == 0 { (o as i64 + idx as i64) as usize } else { idx.max(0) as usize }
        };
        let mut sections = Vec::new();
        if section_frames > 0 && section_index != 0 {
            let n = num_frames / section_frames + 2;
            let base = (o as i64 + section_index as i64) as usize;
            for s in 0..n {
                let block = r.i32(base + s * 8)?;
                let idx = r.i32(base + s * 8 + 4)?;
                sections.push((block, rel(block, idx)));
            }
        }
        out.push(AnimDesc {
            name,
            fps: r.f32(o + 8)?,
            flags: r.i32(o + 12)?,
            num_frames,
            anim_block,
            anim_offset: rel(anim_block, anim_index),
            section_frames,
            sections,
        });
    }
    Ok(out)
}

pub(crate) fn read_sequences(r: &R, count: usize, index: usize, num_bones: usize) -> io::Result<Vec<Sequence>> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = index + i * SEQDESC_STRIDE;
        let label = r.cstr((o as i64 + r.i32(o + 4)? as i64) as usize)?;
        let act_ofs = r.i32(o + 8)?;
        let activity = if act_ofs != 0 { r.cstr((o as i64 + act_ofs as i64) as usize)? } else { String::new() };
        let group_size = [r.i32(o + 68)?.max(1), r.i32(o + 72)?.max(1)];
        let anim_index = (o as i64 + r.i32(o + 60)? as i64) as usize;
        let n = (group_size[0] * group_size[1]).clamp(1, 1024) as usize;
        let mut anims = Vec::with_capacity(n);
        for k in 0..n {
            anims.push(r.i16(anim_index + k * 2)?);
        }
        let weight_index = r.i32(o + 156)?;
        let mut bone_weights = Vec::new();
        if weight_index != 0 {
            let base = (o as i64 + weight_index as i64) as usize;
            for b in 0..num_bones {
                bone_weights.push(r.f32(base + b * 4).unwrap_or(1.0));
            }
        }
        out.push(Sequence {
            name: label,
            activity,
            flags: r.i32(o + 12)?,
            anims,
            group_size,
            param_index: [r.i32(o + 76)?, r.i32(o + 80)?],
            param_start: [r.f32(o + 84)?, r.f32(o + 88)?],
            param_end: [r.f32(o + 92)?, r.f32(o + 96)?],
            fade_in: r.f32(o + 104)?,
            fade_out: r.f32(o + 108)?,
            bone_weights,
        });
    }
    Ok(out)
}

fn f16(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1F) as u32;
    let mant = (bits & 0x3FF) as u32;
    let v = if exp == 0 {
        (mant as f32) * 2f32.powi(-24)
    } else if exp == 31 {
        f32::INFINITY
    } else {
        f32::from_bits(((exp + 112) << 23) | (mant << 13))
    };
    if sign == 1 { -v } else { v }
}

fn quat48(r: &R, o: usize) -> io::Result<Quat> {
    let a = r.u16(o)? as f32;
    let b = r.u16(o + 2)? as f32;
    let c = r.u16(o + 4)?;
    let x = (a - 32768.0) / 32768.0;
    let y = (b - 32768.0) / 32768.0;
    let z = ((c & 0x7FFF) as f32 - 16384.0) / 16384.0;
    let mut w = (1.0 - x * x - y * y - z * z).max(0.0).sqrt();
    if c & 0x8000 != 0 {
        w = -w;
    }
    Ok(Quat::from_xyzw(x, y, z, w))
}

fn quat64(r: &R, o: usize) -> io::Result<Quat> {
    let lo = r.i32(o)? as u32 as u64;
    let hi = r.i32(o + 4)? as u32 as u64;
    let bits = lo | (hi << 32);
    let field = |shift: u32| ((bits >> shift) & 0x1F_FFFF) as f32;
    let x = (field(0) - 1_048_576.0) / 1_048_576.5;
    let y = (field(21) - 1_048_576.0) / 1_048_576.5;
    let z = (field(42) - 1_048_576.0) / 1_048_576.5;
    let mut w = (1.0 - x * x - y * y - z * z).max(0.0).sqrt();
    if bits >> 63 != 0 {
        w = -w;
    }
    Ok(Quat::from_xyzw(x, y, z, w))
}

/// "Smallest three" quaternion in 48 bits (CS:GO frame animations).
///
/// Components are ordered (w, x, y, z). Each 16-bit word holds a 15-bit
/// value in [-1/√2, 1/√2] (offset 16384, scale 1/16384/√2) plus a spare
/// top bit. The top bits of the first two words give the position of the
/// dropped (largest-magnitude) component; the third word's top bit is its
/// sign. The stored values are the components that follow it cyclically.
///
/// Recovered from the game's data: the smallest-three property holds for
/// every sampled rotation; all bit patterns decode continuously into each
/// other where animations switch pattern; and this layout puts every
/// viewmodel's weapon in front of the camera, slightly right and below eye
/// level, for all nine weapons checked.
pub fn quat48s(w0: u16, w1: u16, w2: u16) -> Quat {
    let s = |v: u16| ((v & 0x7FFF) as f32 - 16384.0) / 16384.0 * std::f32::consts::FRAC_1_SQRT_2;
    let (a, b, c) = (s(w0), s(w1), s(w2));
    let mut big = (1.0 - a * a - b * b - c * c).max(0.0).sqrt();
    if w2 & 0x8000 != 0 {
        big = -big;
    }
    let idx = (((w0 >> 15) << 1) | (w1 >> 15)) as usize;
    let mut wxyz = [0.0f32; 4];
    wxyz[idx] = big;
    wxyz[(idx + 1) % 4] = a;
    wxyz[(idx + 2) % 4] = b;
    wxyz[(idx + 3) % 4] = c;
    Quat::from_xyzw(wxyz[1], wxyz[2], wxyz[3], wxyz[0])
}

fn read_quat48s(r: &R, o: usize) -> io::Result<Quat> {
    Ok(quat48s(r.u16(o)?, r.u16(o + 2)?, r.u16(o + 4)?))
}

fn vector48(r: &R, o: usize) -> io::Result<Vec3> {
    Ok(Vec3::new(f16(r.u16(o)?), f16(r.u16(o + 2)?), f16(r.u16(o + 4)?)))
}

/// Frame-animation flags (per bone).
const FRAME_CONST_POS: u8 = 0x01;
const FRAME_CONST_ROT: u8 = 0x02;
const FRAME_ANIM_POS: u8 = 0x04;
const FRAME_ANIM_ROT: u8 = 0x08;
const FRAME_FULL_ANIM_POS: u8 = 0x10;
const FRAME_CONST_ROT2: u8 = 0x40;
const FRAME_ANIM_ROT2: u8 = 0x80;

/// Decodes one frame of the CS:GO frame-animation format.
///
/// Layout (offsets from the record start): i32 constants offset, i32 frame
/// data offset, i32 bytes per frame, 3 unused i32; then one flag byte per
/// bone. Constants (per flagged bone, rotation then position) are stored
/// once; frame data holds the animated values for every frame. Rotations
/// are `Quaternion48` (0x02/0x08) or smallest-three `Quaternion48S`
/// (0x40/0x80); positions are `Vector48` (0x01/0x04) or full floats (0x10).
fn decode_frame_anim(data: &[u8], start: usize, frame: usize, bones: &[Bone], delta: bool, out: &mut [(Vec3, Quat)], animated: &mut [bool]) -> io::Result<()> {
    let r = R(data);
    let const_ofs = r.i32(start)?.max(0) as usize;
    let frame_ofs = r.i32(start + 4)?.max(0) as usize;
    let frame_len = r.i32(start + 8)?.max(0) as usize;
    let mut c = start + const_ofs;
    let mut d = start + frame_ofs + frame * frame_len;
    for (i, b) in bones.iter().enumerate() {
        let f = r.u8(start + 24 + i)?;
        if f != 0 {
            animated[i] = true;
        }
        let (mut pos, mut rot) = if delta { (Vec3::ZERO, Quat::IDENTITY) } else { (b.pos, b.quat) };
        if f & FRAME_CONST_ROT2 != 0 {
            rot = read_quat48s(&r, c)?;
            c += 6;
        } else if f & FRAME_CONST_ROT != 0 {
            rot = quat48(&r, c)?;
            c += 6;
        }
        if f & FRAME_CONST_POS != 0 {
            pos = vector48(&r, c)?;
            c += 6;
        }
        if f & FRAME_ANIM_ROT2 != 0 {
            rot = read_quat48s(&r, d)?;
            d += 6;
        } else if f & FRAME_ANIM_ROT != 0 {
            rot = quat48(&r, d)?;
            d += 6;
        }
        if f & FRAME_ANIM_POS != 0 {
            pos = vector48(&r, d)?;
            d += 6;
        } else if f & FRAME_FULL_ANIM_POS != 0 {
            pos = r.vec3(d)?;
            d += 12;
        }
        out[i] = (pos, rot.normalize());
    }
    Ok(())
}

/// Reads one RLE-compressed value stream at `frame`.
///
/// The stream is a list of runs: a header short (`valid`, `total` bytes)
/// followed by `valid` values; frames past `valid` within a run repeat the
/// last value.
fn rle_value(r: &R, mut p: usize, frame: usize) -> io::Result<f32> {
    let mut k = frame;
    for _ in 0..4096 {
        let valid = r.u8(p)? as usize;
        let total = r.u8(p + 1)? as usize;
        if total == 0 {
            return Err(invalid("animation value run with zero length"));
        }
        if total > k {
            let idx = if valid > k { k + 1 } else { valid };
            return Ok(r.i16(p + idx * 2)? as f32);
        }
        k -= total;
        p += (valid + 1) * 2;
    }
    Err(invalid("animation value stream too long"))
}

/// Three streams located by a `mstudioanim_valueptr_t` (three shorts,
/// offsets relative to the pointer; 0 = axis not animated).
fn value_ptr(r: &R, at: usize, frame: usize, scale: Vec3) -> io::Result<Vec3> {
    let mut out = Vec3::ZERO;
    for axis in 0..3 {
        let ofs = r.i16(at + axis * 2)?;
        if ofs > 0 {
            out[axis] = rle_value(r, at + ofs as usize, frame)? * scale[axis];
        }
    }
    Ok(out)
}

/// Decodes the bone-local pose of one frame of an animation.
///
/// `data` is the buffer the animation lives in (the .mdl, or the .ani for
/// anim blocks) and `start` the offset of the first bone record for the
/// frame's section; `frame` is relative to that section. Bones the
/// animation does not touch keep their rest pose (identity for deltas).
fn decode_frame(data: &[u8], start: usize, frame: usize, bones: &[Bone], delta: bool, out: &mut [(Vec3, Quat)], animated: &mut [bool]) -> io::Result<()> {
    let r = R(data);
    for (i, b) in bones.iter().enumerate() {
        out[i] = if delta { (Vec3::ZERO, Quat::IDENTITY) } else { (b.pos, b.quat) };
    }
    let mut p = start;
    for _ in 0..bones.len().max(1) * 2 {
        let bone = r.u8(p)? as usize;
        let fl = r.u8(p + 1)?;
        let next = r.i16(p + 2)?;
        if bone < bones.len() {
            animated[bone] = true;
            let b = &bones[bone];
            let bone_delta = delta || fl & ANIM_DELTA != 0;
            let mut d = p + 4;
            let mut rot = out[bone].1;
            if fl & ANIM_RAWROT != 0 {
                rot = quat48(&r, d)?;
                d += 6;
            } else if fl & ANIM_RAWROT2 != 0 {
                rot = quat64(&r, d)?;
                d += 8;
            } else if fl & ANIM_ANIMROT != 0 {
                let mut e = value_ptr(&r, d, frame, b.rot_scale)?;
                if !bone_delta {
                    e += b.rot;
                }
                rot = euler_to_quat(e);
                d += 6;
            }
            let mut pos = out[bone].0;
            if fl & ANIM_RAWPOS != 0 {
                pos = Vec3::new(f16(r.u16(d)?), f16(r.u16(d + 2)?), f16(r.u16(d + 4)?));
            } else if fl & ANIM_ANIMPOS != 0 {
                let mut v = value_ptr(&r, d, frame, b.pos_scale)?;
                if !bone_delta {
                    v += b.pos;
                }
                pos = v;
            }
            out[bone] = (pos, rot.normalize());
        }
        if next <= 0 {
            break;
        }
        p += next as usize;
    }
    Ok(())
}

/// A fully decoded animation: every frame's bone-local pose.
#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub fps: f32,
    pub looping: bool,
    pub delta: bool,
    /// `frames[f][bone]`
    pub frames: Vec<Vec<(Vec3, Quat)>>,
    /// Bones the animation carries data for. Others hold their rest value
    /// in `frames`; layered playback may keep the previous pose instead.
    pub animated: Vec<bool>,
}

impl Clip {
    pub fn duration(&self) -> f32 {
        if self.fps > 0.0 && self.frames.len() > 1 { (self.frames.len() - 1) as f32 / self.fps } else { 0.0 }
    }

    /// Bone-local pose at `t` seconds (looped or clamped), interpolated.
    pub fn sample(&self, t: f32, out: &mut [(Vec3, Quat)]) {
        if self.frames.is_empty() {
            return;
        }
        let n = self.frames.len();
        let mut f = t * self.fps.max(1.0);
        let last = (n - 1) as f32;
        if self.looping && last > 0.0 {
            f = f.rem_euclid(last);
        } else {
            f = f.clamp(0.0, last);
        }
        let i0 = f.floor() as usize;
        let i1 = (i0 + 1).min(n - 1);
        let s = f - i0 as f32;
        for (k, slot) in out.iter_mut().enumerate() {
            let (Some(a), Some(b)) = (self.frames[i0].get(k), self.frames[i1].get(k)) else { continue };
            *slot = (a.0.lerp(b.0, s), a.1.slerp(b.1, s));
        }
    }

    /// Like [`Clip::sample`], but only touches bones this clip animates,
    /// blended in by `weight(bone)` (sequence weight lists).
    pub fn sample_layered(&self, t: f32, out: &mut [(Vec3, Quat)], weight: impl Fn(usize) -> f32) {
        let mut full = out.to_vec();
        self.sample(t, &mut full);
        for (i, slot) in out.iter_mut().enumerate() {
            let w = if self.animated.get(i).copied().unwrap_or(false) { weight(i) } else { 0.0 };
            if w >= 0.999 {
                *slot = full[i];
            } else if w > 0.0 {
                *slot = (slot.0.lerp(full[i].0, w), slot.1.slerp(full[i].1, w));
            }
        }
    }
}

/// Re-expresses a clip decoded against `from` on the skeleton `to`,
/// matching bones by name. Bones `to` has but `from` lacks keep their rest
/// pose (or identity for deltas) and are marked as not animated.
pub fn retarget(clip: &Clip, from: &[Bone], to: &[Bone]) -> Clip {
    let map: Vec<Option<usize>> = to
        .iter()
        .map(|b| from.iter().position(|f| f.name.eq_ignore_ascii_case(&b.name)))
        .collect();
    let frames = clip
        .frames
        .iter()
        .map(|f| {
            to.iter()
                .zip(&map)
                .map(|(b, m)| {
                    m.and_then(|i| f.get(i).copied())
                        .unwrap_or(if clip.delta { (Vec3::ZERO, Quat::IDENTITY) } else { (b.pos, b.quat) })
                })
                .collect()
        })
        .collect();
    let animated = map
        .iter()
        .map(|m| m.is_some_and(|i| clip.animated.get(i).copied().unwrap_or(false)))
        .collect();
    Clip {
        name: clip.name.clone(),
        fps: clip.fps,
        looping: clip.looping,
        delta: clip.delta,
        frames,
        animated,
    }
}

/// Retargets per-bone sequence weights by bone name (missing → 1).
pub fn retarget_weights(seq: &Sequence, from: &[Bone], to: &[Bone]) -> Vec<f32> {
    to.iter()
        .map(|b| {
            from.iter()
                .position(|f| f.name.eq_ignore_ascii_case(&b.name))
                .map_or(1.0, |i| seq.weight(i))
        })
        .collect()
}

/// Decodes a whole animation.
pub fn decode_clip(mdl: &[u8], ani: Option<&[u8]>, blocks: &[(u32, u32)], desc: &AnimDesc, bones: &[Bone]) -> io::Result<Clip> {
    let frame_anim = desc.flags & flags::FRAMEANIM != 0;
    let mut animated = vec![false; bones.len()];
    let mut decode = |buf: &[u8], start: usize, frame: usize, pose: &mut [(Vec3, Quat)], delta: bool| {
        if frame_anim {
            decode_frame_anim(buf, start, frame, bones, delta, pose, &mut animated)
        } else {
            decode_frame(buf, start, frame, bones, delta, pose, &mut animated)
        }
    };
    let delta = desc.flags & flags::DELTA != 0;
    let locate = |block: i32, offset: usize| -> io::Result<(&[u8], usize)> {
        if block == 0 {
            Ok((mdl, offset))
        } else {
            let ani = ani.ok_or_else(|| invalid(format!("{} needs its .ani file", desc.name)))?;
            let (start, _) = *blocks
                .get(block as usize)
                .ok_or_else(|| invalid(format!("{}: bad anim block {block}", desc.name)))?;
            Ok((ani, start as usize + offset))
        }
    };
    let frames_total = desc.num_frames.max(1);
    let mut frames = Vec::with_capacity(frames_total);
    let mut pose = vec![(Vec3::ZERO, Quat::IDENTITY); bones.len()];
    for f in 0..frames_total {
        if desc.flags & flags::ALLZEROS != 0 {
            for (i, b) in bones.iter().enumerate() {
                pose[i] = if delta { (Vec3::ZERO, Quat::IDENTITY) } else { (b.pos, b.quat) };
            }
        } else if desc.section_frames > 0 && !desc.sections.is_empty() {
            let s = (f / desc.section_frames).min(desc.sections.len() - 1);
            let (block, ofs) = desc.sections[s];
            let (buf, start) = locate(block, ofs)?;
            decode(buf, start, f - s * desc.section_frames, &mut pose, delta)?;
        } else {
            let (buf, start) = locate(desc.anim_block, desc.anim_offset)?;
            decode(buf, start, f, &mut pose, delta)?;
        }
        frames.push(pose.clone());
    }
    drop(decode);
    Ok(Clip {
        name: desc.name.clone(),
        fps: desc.fps,
        looping: desc.flags & flags::LOOPING != 0,
        delta,
        frames,
        animated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_runs_and_repeats() {
        // Run 1: valid 2, total 3 → values 10, 20, then frame 2 repeats 20.
        // Run 2: valid 1, total 2 → value 30 for frames 3 and 4.
        let mut d = Vec::new();
        d.extend_from_slice(&[2u8, 3]);
        d.extend_from_slice(&10i16.to_le_bytes());
        d.extend_from_slice(&20i16.to_le_bytes());
        d.extend_from_slice(&[1u8, 2]);
        d.extend_from_slice(&30i16.to_le_bytes());
        let r = R(&d);
        let vals: Vec<f32> = (0..5).map(|f| rle_value(&r, 0, f).unwrap()).collect();
        assert_eq!(vals, [10.0, 20.0, 20.0, 30.0, 30.0]);
    }

    #[test]
    fn quat48s_layout() {
        let enc = |v: f32| ((v / std::f32::consts::FRAC_1_SQRT_2) * 16384.0 + 16384.0).round() as u16;
        // Dropped w (position 0), positive: stored values are x, y, z.
        let q = quat48s(enc(0.1), enc(-0.2), enc(0.3));
        assert!((q.x - 0.1).abs() < 1e-3 && (q.y + 0.2).abs() < 1e-3 && (q.z - 0.3).abs() < 1e-3 && q.w > 0.9);
        // Dropped x (position 1), sign bit set: stored values are y, z, w.
        let q = quat48s(enc(0.1), enc(0.2) | 0x8000, enc(0.3) | 0x8000);
        assert!(q.x < -0.9 && (q.y - 0.1).abs() < 1e-3 && (q.z - 0.2).abs() < 1e-3 && (q.w - 0.3).abs() < 1e-3);
    }

    #[test]
    fn quat48s_clean_values() {
        // Raw words seen in v_rif_ak47's idle: all components ±0.5.
        let q = quat48s(0xed40, 0xed3f, 0x6d3f);
        for c in [q.x, q.y, q.z, q.w] {
            assert!((c.abs() - 0.5).abs() < 1e-3, "{q:?}");
        }
        assert!((q.length() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn half_floats() {
        assert_eq!(f16(0x3C00), 1.0);
        assert_eq!(f16(0xC000), -2.0);
    }

    #[test]
    fn quat48_identity() {
        // x = y = z = 0 → raw 32768, 32768, 16384; w positive.
        let mut d = Vec::new();
        d.extend_from_slice(&32768u16.to_le_bytes());
        d.extend_from_slice(&32768u16.to_le_bytes());
        d.extend_from_slice(&16384u16.to_le_bytes());
        let q = quat48(&R(&d), 0).unwrap();
        assert!((q.w - 1.0).abs() < 1e-6 && q.x.abs() < 1e-6);
    }
}
