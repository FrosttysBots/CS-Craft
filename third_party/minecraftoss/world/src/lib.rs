//! The server-side chunk map for MinecraftOSS.
//!
//! Which chunks a player is sent, when they may be sent and in what order
//! follows vanilla 26.3 (`ChunkTrackingView`, `ChunkMap.updateChunkTracking`,
//! `PlayerChunkSender`, the player ticket throttle in `DistanceManager`).
//! How the chunks are produced does not: generation runs on a pool of
//! worker threads with no global ordering between them. Only the order in
//! which work starts and the order in which finished chunks reach the client
//! are observable, and both are kept.

pub mod chunk_map;
pub mod distance;
pub mod level;
pub mod natural_spawner;
pub mod spawn;
pub mod settings;
pub mod storage;
pub mod view;

pub use chunk_map::{ChunkEvent, ChunkMap, ChunkMapStats};
pub use view::TrackingView;
