//! Density functions: parsing, vanilla's optimizer and compiler, and evaluation.

pub mod compile;
pub mod eval;
pub mod ir;
pub mod sampler;

pub use compile::Compiler;
pub use eval::Context;
pub use ir::{Df, Registry};
pub use sampler::{Id, Program};
