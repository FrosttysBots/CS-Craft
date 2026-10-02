//! Anvil region files (`RegionFile`): 32x32 chunks per `r.<x>.<z>.mca`, a
//! sector table and timestamps, then each chunk's compressed NBT in
//! 4096-byte sectors.
//!
//! Chunks are held as their stored (compressed) payloads: a chunk is
//! compressed once when it is saved and decompressed when it is read, and
//! writing a region only lays the payloads out in sectors. The payloads are
//! shared, so a region can be snapshotted under a lock and written, parsed
//! or compressed outside it.

use crate::nbt::{self, Tag};
use crate::pos::ChunkPos;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SECTOR: usize = 4096;

/// One chunk as a region file stores it: its compression type
/// (1 gzip, 2 zlib, 3 none) and compressed body.
#[derive(Clone, Debug)]
pub struct StoredChunk {
    kind: u8,
    body: Arc<[u8]>,
}

impl StoredChunk {
    /// A chunk's NBT compressed as vanilla writes it (zlib, the default level).
    pub fn encode(tag: &Tag) -> Self {
        let data = nbt::write(tag, "");
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::with_capacity(data.len() / 4), flate2::Compression::default());
        encoder.write_all(&data).expect("writing to memory");
        Self { kind: 2, body: encoder.finish().expect("writing to memory").into() }
    }

    /// The uncompressed NBT payload.
    pub fn decompress(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        match self.kind {
            1 => {
                flate2::read::GzDecoder::new(&*self.body).read_to_end(&mut out).map_err(|e| format!("gzip chunk: {e}"))?;
            }
            2 => {
                flate2::read::ZlibDecoder::new(&*self.body).read_to_end(&mut out).map_err(|e| format!("zlib chunk: {e}"))?;
            }
            3 => out.extend_from_slice(&self.body),
            other => return Err(format!("unsupported chunk compression {other}")),
        }
        Ok(out)
    }

    /// The chunk's NBT.
    pub fn parse(&self) -> Result<Tag, String> {
        nbt::parse(&self.decompress()?)
    }
}

/// One region file's chunks, held in memory as their stored payloads.
#[derive(Clone, Default)]
pub struct RegionFile {
    /// Stored chunk and its timestamp, by local index `x + z * 32`.
    chunks: Vec<Option<(StoredChunk, u32)>>,
}

/// The region file name of a chunk and its local index.
pub fn region_of(pos: ChunkPos) -> ((i32, i32), usize) {
    ((pos.x >> 5, pos.z >> 5), ((pos.x & 31) + (pos.z & 31) * 32) as usize)
}

pub fn region_path(dir: &Path, region: (i32, i32)) -> PathBuf {
    dir.join(format!("r.{}.{}.mca", region.0, region.1))
}

impl RegionFile {
    pub fn new() -> Self {
        Self { chunks: vec![None; 1024] }
    }

    /// Reads a region file; zlib, gzip and uncompressed chunks are supported.
    /// Chunks stay compressed until they are read.
    pub fn read(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut region = Self::new();
        if raw.len() < 2 * SECTOR {
            return Ok(region);
        }
        for i in 0..1024 {
            let entry = u32::from_be_bytes(raw[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
            let (offset, count) = ((entry >> 8) as usize, (entry & 0xFF) as usize);
            if offset == 0 || count == 0 {
                continue;
            }
            let timestamp = u32::from_be_bytes(raw[SECTOR + i * 4..SECTOR + i * 4 + 4].try_into().expect("4 bytes"));
            let start = offset * SECTOR;
            let header = raw.get(start..start + 5).ok_or("chunk past end of region file")?;
            let length = u32::from_be_bytes(header[0..4].try_into().expect("4 bytes")) as usize;
            let body = raw.get(start + 5..start + 4 + length).ok_or("chunk body past end of region file")?;
            if !(1..=3).contains(&header[4]) {
                return Err(format!("unsupported chunk compression {}", header[4]));
            }
            region.chunks[i] = Some((StoredChunk { kind: header[4], body: body.into() }, timestamp));
        }
        Ok(region)
    }

    pub fn get(&self, index: usize) -> Result<Option<Tag>, String> {
        self.stored(index).map(|stored| stored.parse()).transpose()
    }

    /// A chunk as stored, to parse elsewhere.
    pub fn stored(&self, index: usize) -> Option<StoredChunk> {
        self.chunks[index].as_ref().map(|(stored, _)| stored.clone())
    }

    /// Whether the region holds a chunk at `index`.
    pub fn contains(&self, index: usize) -> bool {
        self.chunks[index].is_some()
    }

    pub fn set(&mut self, index: usize, tag: &Tag, timestamp: u32) {
        self.set_stored(index, StoredChunk::encode(tag), timestamp);
    }

    /// Sets an already compressed chunk.
    pub fn set_stored(&mut self, index: usize, stored: StoredChunk, timestamp: u32) {
        self.chunks[index] = Some((stored, timestamp));
    }

    pub fn remove(&mut self, index: usize) {
        self.chunks[index] = None;
    }

    /// Writes the region with its chunks packed from sector 2, written to a
    /// temporary file first and then moved into place.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let mut locations = vec![0u8; SECTOR];
        let mut timestamps = vec![0u8; SECTOR];
        let mut body = Vec::new();
        let mut sector = 2usize;
        for (i, chunk) in self.chunks.iter().enumerate() {
            let Some((stored, timestamp)) = chunk else { continue };
            let length = stored.body.len() + 5;
            let sectors = length.div_ceil(SECTOR);
            if sectors > 255 {
                return Err(format!("chunk {i} needs {sectors} sectors; external chunk files are not supported"));
            }
            body.extend_from_slice(&((stored.body.len() + 1) as u32).to_be_bytes());
            body.push(stored.kind);
            body.extend_from_slice(&stored.body);
            body.resize(body.len() + sectors * SECTOR - length, 0);
            locations[i * 4..i * 4 + 4].copy_from_slice(&(((sector as u32) << 8) | sectors as u32).to_be_bytes());
            timestamps[i * 4..i * 4 + 4].copy_from_slice(&timestamp.to_be_bytes());
            sector += sectors;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let temp = path.with_extension("mca.tmp");
        let mut file = std::fs::File::create(&temp).map_err(|e| format!("{}: {e}", temp.display()))?;
        file.write_all(&locations).and_then(|_| file.write_all(&timestamps)).and_then(|_| file.write_all(&body)).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&temp, path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_survive_a_write_and_read() {
        let dir = std::env::temp_dir().join(format!("minecraftoss-anvil-{}", std::process::id()));
        let path = region_path(&dir, (0, 0));
        let mut region = RegionFile::new();
        let tag = Tag::Compound([("Status".to_owned(), Tag::String("minecraft:full".into())), ("xPos".to_owned(), Tag::Int(3))].into_iter().collect());
        region.set(3, &tag, 7);
        region.set_stored(40, StoredChunk::encode(&Tag::Compound([("zPos".to_owned(), Tag::Int(1))].into_iter().collect())), 9);
        region.write(&path).unwrap();
        let read = RegionFile::read(&path).unwrap();
        assert_eq!(read.get(3).unwrap(), Some(tag));
        assert!(read.contains(40) && !read.contains(41));
        assert_eq!(read.get(41).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
