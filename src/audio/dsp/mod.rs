//! The built-in processors: what [`create`](crate::audio::processor::create) makes by kind.
//!
//! Each is a [`Processor`](crate::audio::Processor) that allocates everything when it's made, so
//! processing a block touches only memory it already holds.

// Signal processing is sums of products throughout. `mul_add` is a libm call on a CPU built for
// without FMA, many times slower than the plain expression, and rounding once instead of twice
// buys nothing audible.
#![allow(clippy::suboptimal_flops)]

pub mod delay;
pub mod dynamics;
pub mod filter;
pub mod pan;
pub mod reverb;
