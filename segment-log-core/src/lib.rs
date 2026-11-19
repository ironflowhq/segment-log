// The README is the crate's front page on docs.rs; its links are absolute so they resolve there.
#![doc = include_str!("../README.md")]
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
/// The operator repair tool's core (ADR 0065). Not part of the API promise: use
/// `segment-log-server repair`.
#[doc(hidden)]
pub use engine::repair;
pub use engine::{LogReader, Pending, SegmentLog};

#[cfg(test)]
mod pipeline;
