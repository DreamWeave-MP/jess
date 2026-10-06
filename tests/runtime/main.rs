//! The runtime's integration tests, linked as one binary: every binary pays l3i's link-time
//! codegen, so one is cheaper than five.

#[path = "fixture.rs"]
mod fixture;
#[path = "lifecycle.rs"]
mod lifecycle;
#[path = "scheduling.rs"]
mod scheduling;
#[path = "services.rs"]
mod services;
#[path = "support.rs"]
mod support;
#[path = "workers.rs"]
mod workers;
