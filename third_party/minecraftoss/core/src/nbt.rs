//! Named Binary Tag reading and writing: gzip-compressed or raw big-endian
//! NBT, as used by structure templates and saved chunks.
//!
//! Lists may mix element types (26.3 `ListTag`): on disk such a list is a
//! compound list whose non-compound elements, and compounds that look like
//! wrappers, sit in `{"": element}` wrappers. Reading unwraps them, so a
//! `Tag::List` holds the logical elements; writing wraps them again.

use std::collections::BTreeMap;
use std::io::Read;

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Tag>),
    Compound(BTreeMap<String, Tag>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Self::Compound(map) => map.get(key),
            _ => None,
        }
    }

    pub fn as_compound(&self) -> Option<&BTreeMap<String, Tag>> {
        match self {
            Self::Compound(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Tag]> {
        match self {
            Self::List(list) => Some(list),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// Any integer tag widened to `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        Some(match *self {
            Self::Byte(v) => i64::from(v),
            Self::Short(v) => i64::from(v),
            Self::Int(v) => i64::from(v),
            Self::Long(v) => v,
            _ => return None,
        })
    }

    pub fn as_f64(&self) -> Option<f64> {
        Some(match *self {
            Self::Float(v) => f64::from(v),
            Self::Double(v) => v,
            _ => return self.as_i64().map(|v| v as f64),
        })
    }

    /// An int list or int array as `i32`s (block positions and sizes).
    pub fn as_ints(&self) -> Option<Vec<i32>> {
        match self {
            Self::IntArray(v) => Some(v.clone()),
            Self::List(list) => list.iter().map(|t| t.as_i64().map(|v| v as i32)).collect(),
            _ => None,
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or("truncated NBT")?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn i16(&mut self) -> Result<i16, String> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().expect("2 bytes")))
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().expect("8 bytes")))
    }

    fn len(&mut self) -> Result<usize, String> {
        let n = self.i32()?;
        usize::try_from(n).map_err(|_| format!("negative NBT length {n}"))
    }

    /// Java's modified UTF-8 (`DataInput.readUTF`).
    fn string(&mut self) -> Result<String, String> {
        let n = self.i16()? as u16 as usize;
        let bytes = self.take(n)?;
        let mut units: Vec<u16> = Vec::with_capacity(n);
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            let unit = if b & 0x80 == 0 {
                i += 1;
                u16::from(b)
            } else if b & 0xE0 == 0xC0 {
                let c = *bytes.get(i + 1).ok_or("bad modified UTF-8")?;
                i += 2;
                (u16::from(b & 0x1F) << 6) | u16::from(c & 0x3F)
            } else {
                let c = *bytes.get(i + 1).ok_or("bad modified UTF-8")?;
                let d = *bytes.get(i + 2).ok_or("bad modified UTF-8")?;
                i += 3;
                (u16::from(b & 0x0F) << 12) | (u16::from(c & 0x3F) << 6) | u16::from(d & 0x3F)
            };
            units.push(unit);
        }
        Ok(String::from_utf16_lossy(&units))
    }

    fn payload(&mut self, kind: u8, depth: usize) -> Result<Tag, String> {
        if depth > 512 {
            return Err("NBT nested too deeply".into());
        }
        Ok(match kind {
            1 => Tag::Byte(self.u8()? as i8),
            2 => Tag::Short(self.i16()?),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(self.i64()?),
            5 => Tag::Float(f32::from_bits(self.i32()? as u32)),
            6 => Tag::Double(f64::from_bits(self.i64()? as u64)),
            7 => {
                let n = self.len()?;
                Tag::ByteArray(self.take(n)?.iter().map(|&b| b as i8).collect())
            }
            8 => Tag::String(self.string()?),
            9 => {
                let element = self.u8()?;
                let n = self.len()?;
                let mut list = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    let value = self.payload(element, depth + 1)?;
                    list.push(if element == 10 { unwrap_element(value) } else { value });
                }
                Tag::List(list)
            }
            10 => {
                let mut map = BTreeMap::new();
                loop {
                    let kind = self.u8()?;
                    if kind == 0 {
                        break;
                    }
                    let name = self.string()?;
                    map.insert(name, self.payload(kind, depth + 1)?);
                }
                Tag::Compound(map)
            }
            11 => {
                let n = self.len()?;
                Tag::IntArray((0..n).map(|_| self.i32()).collect::<Result<_, _>>()?)
            }
            12 => {
                let n = self.len()?;
                Tag::LongArray((0..n).map(|_| self.i64()).collect::<Result<_, _>>()?)
            }
            other => return Err(format!("unknown NBT tag type {other}")),
        })
    }
}

/// `ListTag.tryUnwrap`: a compound whose only key is empty stands for its value.
fn unwrap_element(tag: Tag) -> Tag {
    match tag {
        Tag::Compound(mut map) if map.len() == 1 && map.contains_key("") => map.remove("").expect("checked"),
        other => other,
    }
}

impl Tag {
    /// NBT type ID.
    pub fn id(&self) -> u8 {
        match self {
            Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::List(_) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }

    fn is_wrapper(&self) -> bool {
        matches!(self, Self::Compound(map) if map.len() == 1 && map.contains_key(""))
    }
}

/// Java's modified UTF-8 (`DataOutput.writeUTF`).
fn write_string(out: &mut Vec<u8>, s: &str) {
    let mut bytes = Vec::with_capacity(s.len());
    for unit in s.encode_utf16() {
        match unit {
            0x0001..=0x007F => bytes.push(unit as u8),
            0x0000 | 0x0080..=0x07FF => {
                bytes.push(0xC0 | (unit >> 6) as u8);
                bytes.push(0x80 | (unit & 0x3F) as u8);
            }
            _ => {
                bytes.push(0xE0 | (unit >> 12) as u8);
                bytes.push(0x80 | ((unit >> 6) & 0x3F) as u8);
                bytes.push(0x80 | (unit & 0x3F) as u8);
            }
        }
    }
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(&bytes);
}

fn write_payload(out: &mut Vec<u8>, tag: &Tag) {
    match tag {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Tag::ByteArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            out.extend(v.iter().map(|&b| b as u8));
        }
        Tag::String(s) => write_string(out, s),
        Tag::List(list) => {
            // `ListTag.identifyRawElementType`: one type, or wrapped compounds.
            let first = list.first().map(Tag::id);
            let uniform = list.iter().all(|t| Some(t.id()) == first) && !list.iter().any(Tag::is_wrapper);
            let element = if list.is_empty() { 0 } else if uniform { first.expect("non-empty") } else { 10 };
            out.push(element);
            out.extend_from_slice(&(list.len() as i32).to_be_bytes());
            for item in list {
                if uniform || matches!(item, Tag::Compound(_)) && !item.is_wrapper() {
                    write_payload(out, item);
                } else {
                    out.push(item.id());
                    write_string(out, "");
                    write_payload(out, item);
                    out.push(0);
                }
            }
        }
        Tag::Compound(map) => {
            for (key, value) in map {
                out.push(value.id());
                write_string(out, key);
                write_payload(out, value);
            }
            out.push(0);
        }
        Tag::IntArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
        Tag::LongArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
    }
}

/// Writes an uncompressed NBT document with one named root tag.
pub fn write(tag: &Tag, name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(tag.id());
    write_string(&mut out, name);
    write_payload(&mut out, tag);
    out
}

/// Writes a gzip-compressed NBT document (`NbtIo.writeCompressed`).
pub fn write_gzip(tag: &Tag, name: &str) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&write(tag, name)).expect("in-memory write");
    encoder.finish().expect("in-memory write")
}

/// Parses an uncompressed NBT document: one named root tag.
pub fn parse(data: &[u8]) -> Result<Tag, String> {
    let mut reader = Reader { data, pos: 0 };
    let kind = reader.u8()?;
    let _name = reader.string()?;
    reader.payload(kind, 0)
}

/// Reads gzip-compressed NBT (or raw NBT when the data is not gzip).
pub fn read(bytes: &[u8]) -> Result<Tag, String> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes).read_to_end(&mut out).map_err(|e| format!("gzip: {e}"))?;
        parse(&out)
    } else {
        parse(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_small_compound() {
        // {"": {a: 1b, s: "hi", l: [1, 2] (ints)}}
        let mut data = vec![10, 0, 0];
        data.extend([1, 0, 1, b'a', 1]);
        data.extend([8, 0, 1, b's', 0, 2, b'h', b'i']);
        data.extend([9, 0, 1, b'l', 3, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2]);
        data.push(0);
        let tag = parse(&data).unwrap();
        assert_eq!(tag.get("a"), Some(&Tag::Byte(1)));
        assert_eq!(tag.get("s").and_then(Tag::as_str), Some("hi"));
        assert_eq!(tag.get("l").and_then(Tag::as_ints), Some(vec![1, 2]));
        assert_eq!(parse(&write(&tag, "")).unwrap(), tag);
    }

    #[test]
    fn mixed_lists_round_trip_through_wrappers() {
        let mut properties = BTreeMap::new();
        properties.insert("id".to_owned(), Tag::String("minecraft:stairs".into()));
        let mixed = Tag::List(vec![Tag::String("minecraft:stone".into()), Tag::Compound(properties), Tag::Int(3)]);
        let mut root = BTreeMap::new();
        root.insert("palette".to_owned(), mixed);
        let root = Tag::Compound(root);
        let bytes = write(&root, "");
        // The list is written as compounds: type 10 after the list tag's name.
        assert_eq!(bytes[3 + 1 + 2 + 7], 10);
        assert_eq!(parse(&bytes).unwrap(), root);
    }
}
