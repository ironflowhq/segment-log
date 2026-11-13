//! Embedded append-only segment log foundations.
#![forbid(unsafe_code)]

mod codec;
mod storage;

#[cfg(test)]
mod acceptance;
mod api;
pub use api::*;
mod engine;
#[cfg(fuzzing)]
#[doc(hidden)]
pub use engine::fuzzing;
pub use engine::{LogReader, Pending, SegmentLog};

#[cfg(test)]
mod pipeline;
