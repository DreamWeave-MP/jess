//! The runtime's integration tests, linked as one binary: every binary pays l3i's link-time
//! codegen, so one is cheaper than five.

mod events;
mod fixture;
mod lifecycle;
mod scheduling;
mod services;
mod support;
mod workers;
