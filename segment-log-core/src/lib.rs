//! Embedded append-only segment log foundations.
#![forbid(unsafe_code)]

mod codec;
mod storage;

#[cfg(test)]
mod acceptance;
mod api;
pub use api::*;
mod engine;
pub use engine::{Pending, SegmentLog};

#[cfg(test)]
mod pipeline;
