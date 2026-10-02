//! Runtime world generation for MinecraftOSS.
//!
//! Source-informed from the pinned 26.3 common JAR (see
//! `docs/engine-architecture.md`). Numerical behavior follows vanilla
//! exactly, including where its batched and per-point paths differ; the
//! structure is our own. The `engine/worldgen` crate remains the reference
//! oracle this crate is tested against, alongside recorded vanilla captures.

pub mod biome_source;
pub mod carver;
pub mod density;
pub mod feature;
pub mod aquifer;
pub mod interval;
pub mod material;
pub mod mth;
pub mod noise;
pub mod profile;
pub mod providers;
pub mod structure;
pub mod temperature;
pub mod terrain;
pub mod zoom;
