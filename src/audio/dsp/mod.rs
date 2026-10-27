//! The built-in processors: what [`Registry::builtin`](crate::audio::Registry::builtin) makes by kind.
//!
//! Each is a [`Processor`](crate::audio::Processor) that allocates everything when it's made, so
//! processing a block touches only memory it already holds.

// Signal processing is sums of products throughout. `mul_add` is a libm call on a CPU built for
// without FMA, many times slower than the plain expression, and rounding once instead of twice
// buys nothing audible.
#![allow(clippy::suboptimal_flops)]

pub mod crossfade;
pub mod delay;
pub mod drive;
pub mod dynamics;
pub mod filter;
pub mod generators;
pub mod pan;
pub mod reverb;
