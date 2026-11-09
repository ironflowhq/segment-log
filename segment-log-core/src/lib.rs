//! Embedded append-only segment log foundations.
#![forbid(unsafe_code)]

// Slice 0 foundations are private until the engine consumes them.
#[cfg_attr(not(test), allow(dead_code))]
mod codec;
#[cfg_attr(not(test), allow(dead_code))]
mod storage;

#[cfg(test)]
mod acceptance;
