//! The action graph: bindings as structure.
//!
//! [`spec`] is the document an author writes and a user's remaps are saved in. [`compile`] checks
//! it and flattens it. [`eval`] runs the result against device state, once per tick.

pub mod compile;
pub mod eval;
pub mod spec;
