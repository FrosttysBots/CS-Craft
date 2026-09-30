//! Reader for Valve VPK archives (versions 1 and 2).
//!
//! A VPK set is one `*_dir.vpk` holding the directory tree (and sometimes
//! small files) plus numbered data archives `*_000.vpk`, `*_001.vpk`, ...
//!
//! Layout of the directory file:
//! * header: signature `0x55AA1234`, version, tree size (v2 adds four more
//!   section sizes, 28 bytes total)
//! * tree: for each extension, for each directory, for each file name
//!   (all NUL-terminated, lists ended by an empty string) an entry of
//!   CRC32, preload size, archive index, offset, length, `0xFFFF`,
//!   followed by the preload bytes
//!
//! Archive index `0x7FFF` means the data lives in the directory file itself,
//! after the tree.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SIGNATURE: u32 = 0x55AA_1234;
const DIR_ARCHIVE: u16 = 0x7FFF;

#[derive(Clone, Debug)]
pub struct Entry {
    pub crc: u32,
    pub preload: Vec<u8>,
    pub archive_index: u16,
    pub offset: u32,
    pub length: u32,
}

impl Entry {
    pub fn size(&self) -> usize {
        self.preload.len() + self.length as usize
    }
}

pub struct Vpk {
    /// Path of the `_dir.vpk` file.
    dir_path: PathBuf,
    /// `pak01` from `pak01_dir.vpk`, used to build archive file names.
    stem: String,
    /// Offset in the dir file where embedded data starts.
    embedded_base: u64,
    entries: HashMap<String, Entry>,
    handles: Mutex<HashMap<u16, File>>,
}

impl std::fmt::Debug for Vpk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vpk")
            .field("dir_path", &self.dir_path)
            .field("entries", &self.entries.len())
            .finish()
    }
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Normalises a path for lookup: lowercase, forward slashes, no leading `/`.
pub fn normalize(path: &str) -> String {
    let mut s = path.replace('\\', "/").to_ascii_lowercase();
    while s.starts_with('/') {
        s.remove(0);
    }
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    s
}

impl Vpk {
    /// Opens `something_dir.vpk`.
    pub fn open(dir_path: impl AsRef<Path>) -> io::Result<Self> {
        let dir_path = dir_path.as_ref().to_path_buf();
        let bytes = std::fs::read(&dir_path)?;
        let file_name = dir_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| invalid("bad vpk file name"))?;
        let stem = file_name
            .strip_suffix("_dir.vpk")
            .ok_or_else(|| invalid(format!("{file_name} is not a _dir.vpk")))?
            .to_string();
        let (entries, embedded_base) = parse_directory(&bytes)?;
        Ok(Self {
            dir_path,
            stem,
            embedded_base,
            entries,
            handles: Mutex::new(HashMap::new()),
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(&normalize(path))
    }

    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries.get(&normalize(path))
    }

    /// All file paths in the archive (normalised).
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    fn archive_path(&self, index: u16) -> PathBuf {
        if index == DIR_ARCHIVE {
            self.dir_path.clone()
        } else {
            self.dir_path
                .with_file_name(format!("{}_{:03}.vpk", self.stem, index))
        }
    }

    /// Reads a whole file, or `None` if the archive does not contain it.
    pub fn read(&self, path: &str) -> io::Result<Option<Vec<u8>>> {
        let Some(entry) = self.entries.get(&normalize(path)) else {
            return Ok(None);
        };
        let mut out = Vec::with_capacity(entry.size());
        out.extend_from_slice(&entry.preload);
        if entry.length > 0 {
            let offset = if entry.archive_index == DIR_ARCHIVE {
                self.embedded_base + entry.offset as u64
            } else {
                entry.offset as u64
            };
            let mut handles = self.handles.lock().unwrap_or_else(|e| e.into_inner());
            let file = match handles.entry(entry.archive_index) {
                std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(File::open(self.archive_path(entry.archive_index))?)
                }
            };
            file.seek(SeekFrom::Start(offset))?;
            let start = out.len();
            out.resize(start + entry.length as usize, 0);
            file.read_exact(&mut out[start..])?;
        }
        Ok(Some(out))
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn u16(&mut self) -> io::Result<u16> {
        let b = self
            .data
            .get(self.pos..self.pos + 2)
            .ok_or_else(|| invalid("truncated vpk tree"))?;
        self.pos += 2;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> io::Result<u32> {
        let b = self
            .data
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| invalid("truncated vpk tree"))?;
        self.pos += 4;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn cstr(&mut self) -> io::Result<&'a str> {
        let rest = &self.data[self.pos..];
        let len = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| invalid("unterminated string in vpk tree"))?;
        self.pos += len + 1;
        std::str::from_utf8(&rest[..len]).map_err(|_| invalid("non-utf8 name in vpk tree"))
    }

    fn bytes(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let b = self
            .data
            .get(self.pos..self.pos + n)
            .ok_or_else(|| invalid("truncated vpk preload"))?;
        self.pos += n;
        Ok(b)
    }
}

fn parse_directory(bytes: &[u8]) -> io::Result<(HashMap<String, Entry>, u64)> {
    let mut c = Cursor {
        data: bytes,
        pos: 0,
    };
    if c.u32()? != SIGNATURE {
        return Err(invalid("not a vpk (bad signature)"));
    }
    let version = c.u32()?;
    let tree_size = c.u32()? as usize;
    let header_size = match version {
        1 => 12,
        2 => {
            c.u32()?; // file data section size
            c.u32()?; // archive md5 section size
            c.u32()?; // other md5 section size
            c.u32()?; // signature section size
            28
        }
        v => return Err(invalid(format!("unsupported vpk version {v}"))),
    };
    let tree_end = header_size + tree_size;
    if tree_end > bytes.len() {
        return Err(invalid("vpk tree runs past end of file"));
    }

    let mut entries = HashMap::new();
    loop {
        let ext = c.cstr()?;
        if ext.is_empty() {
            break;
        }
        loop {
            let dir = c.cstr()?;
            if dir.is_empty() {
                break;
            }
            loop {
                let name = c.cstr()?;
                if name.is_empty() {
                    break;
                }
                let crc = c.u32()?;
                let preload_len = c.u16()? as usize;
                let archive_index = c.u16()?;
                let offset = c.u32()?;
                let length = c.u32()?;
                let terminator = c.u16()?;
                if terminator != 0xFFFF {
                    return Err(invalid("bad vpk entry terminator"));
                }
                let preload = c.bytes(preload_len)?.to_vec();
                let mut path = String::new();
                if dir != " " {
                    path.push_str(dir);
                    path.push('/');
                }
                path.push_str(name);
                if ext != " " {
                    path.push('.');
                    path.push_str(ext);
                }
                entries.insert(
                    normalize(&path),
                    Entry {
                        crc,
                        preload,
                        archive_index,
                        offset,
                        length,
                    },
                );
            }
        }
    }
    Ok((entries, tree_end as u64))
}

/// Builds an in-memory v2 VPK directory (all data embedded). Used by tests
/// here and in dependent crates.
pub fn build_embedded(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::collections::BTreeMap;
    // ext -> dir -> [(name, data)]
    let mut tree: BTreeMap<String, BTreeMap<String, Vec<(String, &[u8])>>> = BTreeMap::new();
    for (path, data) in files {
        let path = normalize(path);
        let (dir, file) = match path.rfind('/') {
            Some(i) => (path[..i].to_string(), path[i + 1..].to_string()),
            None => (" ".to_string(), path.clone()),
        };
        let (name, ext) = match file.rfind('.') {
            Some(i) => (file[..i].to_string(), file[i + 1..].to_string()),
            None => (file.clone(), " ".to_string()),
        };
        tree.entry(ext)
            .or_default()
            .entry(dir)
            .or_default()
            .push((name, data));
    }
    let mut t = Vec::new();
    let mut blob = Vec::new();
    for (ext, dirs) in &tree {
        t.extend_from_slice(ext.as_bytes());
        t.push(0);
        for (dir, names) in dirs {
            t.extend_from_slice(dir.as_bytes());
            t.push(0);
            for (name, data) in names {
                t.extend_from_slice(name.as_bytes());
                t.push(0);
                t.extend_from_slice(&0u32.to_le_bytes());
                t.extend_from_slice(&0u16.to_le_bytes());
                t.extend_from_slice(&DIR_ARCHIVE.to_le_bytes());
                t.extend_from_slice(&(blob.len() as u32).to_le_bytes());
                t.extend_from_slice(&(data.len() as u32).to_le_bytes());
                t.extend_from_slice(&0xFFFFu16.to_le_bytes());
                blob.extend_from_slice(data);
            }
            t.push(0);
        }
        t.push(0);
    }
    t.push(0);
    let mut out = Vec::new();
    out.extend_from_slice(&SIGNATURE.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(t.len() as u32).to_le_bytes());
    out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&t);
    out.extend_from_slice(&blob);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_embedded() {
        let files: &[(&str, &[u8])] = &[
            ("materials/Brick/Wall01.vmt", b"LightmappedGeneric {}"),
            ("scripts/items/items_game.txt", b"\"items_game\" {}"),
            ("readme", b"no extension"),
        ];
        let bytes = build_embedded(files);
        let dir = std::env::temp_dir().join(format!("vpk_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pak01_dir.vpk");
        std::fs::write(&path, bytes).unwrap();
        let vpk = Vpk::open(&path).unwrap();
        assert_eq!(vpk.len(), 3);
        assert_eq!(
            vpk.read("MATERIALS\\brick\\wall01.vmt").unwrap().unwrap(),
            b"LightmappedGeneric {}"
        );
        assert_eq!(vpk.read("readme").unwrap().unwrap(), b"no extension");
        assert!(vpk.read("missing.txt").unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize("\\Materials//X\\Y.VMT"), "materials/x/y.vmt");
    }
}
