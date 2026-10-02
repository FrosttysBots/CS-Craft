//! Shared foundation for the MinecraftOSS engine crates.
//!
//! Block states are dense numeric IDs in vanilla's global registry order,
//! loaded from the pinned block-state catalog. Tags, biomes and dimension
//! types come from an external data pack. Nothing here hardcodes vanilla
//! data; see `docs/engine-architecture.md`.

pub mod biome;
pub mod block;
pub mod block_entity;
pub mod anvil;
pub mod chunk;
pub mod chunk_nbt;
pub mod datapack;
pub mod entity_data;
pub mod environment;
pub mod ident;
pub mod item;
pub mod light;
pub mod loot;
pub mod nbt;
pub mod fast_hash;
pub mod thread_priority;
pub mod palette;
pub mod pos;
pub mod random;
pub mod registries;
pub mod seed;
pub mod snbt;
pub mod tags;

pub use biome::{BiomeId, BiomeRegistry};
pub use block::{BlockId, BlockRegistry, BlockStateId, FaceShape, SupportType};
pub use chunk::{Chunk, ChunkSection, HeightmapKind, Heightmaps};
pub use ident::Identifier;
pub use pos::{BlockPos, ChunkPos, SectionPos};
pub use registries::Registries;
