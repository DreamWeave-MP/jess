//! The jess application kernel: one [`ScriptDomain`] with its live services, ticked by a host.
//!
//! `jess::runtime` runs script code and knows nothing about what an application is made of.
//! `jess::input` knows devices and action graphs and nothing about domains. This module is where
//! they meet: a [`Host`] creates a domain from any [`RuntimeFactory`] (`jess::lua::plan` is one),
//! attaches the application's input as a service the domain carries ([`DomainSpec::service`]),
//! and drives one frame at a time:
//!
//! 1. **Input.** The host polls the platform backend into the domain's [`InputService`], ticks
//!    it once at the host's clock, and, if the tick produced action events, runs the domain's
//!    [`Phase::Input`](crate::runtime::Phase::Input) with them and the intents routed from them as the payload
//!    ([`input::payload`]). A quiet tick enters no Luau.
//! 2. **Advance.** [`ScriptDomain::advance`]: fixed steps, update, late update, and the event
//!    drain.
//!
//! The host declares no contexts and reserves no action names: the application registers its
//! contexts on [`Host::domain`] and its scripts bind them. What the host owns is what outlives a
//! script generation: the devices and the clock. At a [`Host::restart`] the input
//! service forgets the graph and routes the old code installed, keeps its devices, and the new
//! code installs its own.
//!
//! Everything here runs headless. There is no window, no renderer, and no GUI; those are layers
//! above, and they will stand on this.
//!
//! # Threads
//!
//! A [`Host`] lives on the thread that created it, like its domain:
//!
//! ```compile_fail
//! fn assert_send<T: Send>() {}
//! assert_send::<jess::host::Host>();
//! ```
//!
//! Its input service is shared: `Arc<Mutex<InputService>>` crosses to a backend's poll thread or
//! a worker lane. Localization is not a host service: `@jess/l10n` keeps a registry per runtime.

#![warn(missing_docs)]

pub mod input;

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::input::{BackendError, InputService};
use crate::runtime::l3i::Runtime;
use crate::runtime::{
    DomainSpec, Generation, PhaseReport, RuntimeError, RuntimeFactory, ScriptDomain, TeardownReport, TickReport,
};

pub use input::InputSource;

/// What a host fails with: the runtime's errors, or the input backend's.
#[derive(Debug)]
pub enum Error {
    /// The domain refused something, or a native fault under it.
    Runtime(RuntimeError),
    /// The platform's device layer could not be opened or polled.
    Backend(BackendError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Runtime(error) => error.fmt(f),
            Error::Backend(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<RuntimeError> for Error {
    fn from(error: RuntimeError) -> Self {
        Error::Runtime(error)
    }
}

impl From<BackendError> for Error {
    fn from(error: BackendError) -> Self {
        Error::Backend(error)
    }
}

/// The service type a domain carries for its input: what [`Host`] ticks and `jess.input` binds
/// to. The type is the contract; a binding looks it up with
/// `jess::runtime::service::<SharedInput>()`.
pub type SharedInput = Mutex<InputService>;

/// What a host is made of: the domain's spec and the services it will carry.
pub struct HostSpec {
    domain: DomainSpec,
    input: Option<InputSource>,
}

impl fmt::Debug for HostSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostSpec")
            .field("domain", &self.domain)
            .field("input", &self.input)
            .finish()
    }
}

impl HostSpec {
    /// A host around a domain built from `domain`, with no services until the builder adds them.
    #[must_use]
    pub fn new(domain: DomainSpec) -> HostSpec {
        HostSpec { domain, input: None }
    }

    /// Gives the domain a live input service fed from `source`.
    #[must_use]
    pub fn input(mut self, source: InputSource) -> HostSpec {
        self.input = Some(source);
        self
    }
}

/// What one [`Host::frame`] did.
#[derive(Debug)]
pub struct FrameReport {
    /// The input phase, if the tick produced action events.
    pub input: Option<PhaseReport>,
    /// The domain's advance.
    pub tick: TickReport,
}

impl FrameReport {
    /// Every handler failure of the frame, in the order they happened.
    #[must_use]
    pub fn failures(&self) -> Vec<&RuntimeError> {
        self.input
            .iter()
            .flat_map(|phase| &phase.failures)
            .chain(self.tick.failures())
            .collect()
    }
}

/// One application: a domain and the services the host drives for it. See the module docs.
pub struct Host {
    domain: ScriptDomain,
    input: Option<input::Driver>,
}

impl fmt::Debug for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Host")
            .field("domain", &self.domain)
            .field("input", &self.input.as_ref().map(input::Driver::source_name))
            .finish()
    }
}

impl Host {
    /// Creates the domain from `factory` with the services `spec` asks for attached, and leaves
    /// it `Created`: the application registers contexts, loads code, and starts it through
    /// [`Host::domain`].
    ///
    /// # Errors
    ///
    /// Whatever [`ScriptDomain::create`] returns.
    pub fn create(factory: Arc<dyn RuntimeFactory>, spec: HostSpec) -> Result<Host, RuntimeError> {
        let mut domain_spec = spec.domain;
        let input = spec.input.map(input::Driver::new);
        if let Some(driver) = &input {
            domain_spec = domain_spec.service(Arc::clone(driver.service()));
        }
        let domain = ScriptDomain::create(factory, domain_spec)?;
        Ok(Host { domain, input })
    }

    /// The domain: register contexts, load, start, spawn workers, and inspect through it.
    #[must_use]
    pub fn domain(&self) -> &ScriptDomain {
        &self.domain
    }

    /// The domain's input service, if the host has one: the synthetic side for tests, the graph
    /// and routes for a host that configures input itself.
    #[must_use]
    pub fn input(&self) -> Option<&Arc<SharedInput>> {
        self.input.as_ref().map(input::Driver::service)
    }

    /// One frame: the input tick and phase, then the domain's advance by
    /// `delta`.
    ///
    /// `delta` is the domain's time only. With [`InputSource::Synthetic`] the input ticks at the
    /// synthetic clock as it stands, and the frame does not move it; see that variant.
    ///
    /// # Errors
    ///
    /// As [`ScriptDomain::run_phase`] and [`ScriptDomain::advance`] (the domain is not running,
    /// or a handler of it is), or the backend failed to poll. Handler failures are in the
    /// report, not here.
    pub fn frame(&mut self, delta: Duration) -> Result<FrameReport, Error> {
        let input = self.pump_input()?;
        let tick = self.domain.advance(delta)?;
        Ok(FrameReport { input, tick })
    }

    /// The input step alone: polls the backend, ticks the service, and runs the input phase if
    /// the tick produced action events. `None` when the host has no input or the tick was quiet.
    ///
    /// # Errors
    ///
    /// As [`ScriptDomain::run_phase`], or the backend failed to poll.
    pub fn pump_input(&mut self) -> Result<Option<PhaseReport>, Error> {
        let Some(driver) = &mut self.input else {
            return Ok(None);
        };
        let Some(payload) = driver.tick(&self.domain)? else {
            return Ok(None);
        };
        Ok(Some(self.domain.run_phase(crate::runtime::Phase::Input, &payload)?))
    }

    /// Restarts the domain as a new generation ([`ScriptDomain::restart`]). Before it, the input
    /// service forgets the graph and the routes the old code installed; devices and the queue stay,
    /// since neither restarted.
    ///
    /// Destructive, like the domain's: the old generation and its input configuration are gone
    /// before the new code loads, and a failed load leaves nothing running. Not a last-known-good
    /// reload.
    ///
    /// # Errors
    ///
    /// As [`ScriptDomain::restart`].
    pub fn restart(&mut self, load: impl FnOnce(&Runtime) -> l3i::Result<()>) -> Result<Generation, RuntimeError> {
        if let Some(driver) = &self.input {
            driver.reset();
        }
        self.domain.restart(load)
    }

    /// Tears the domain down ([`ScriptDomain::stop`]) and drops the services with the host.
    ///
    /// # Errors
    ///
    /// As [`ScriptDomain::stop`].
    pub fn stop(self) -> Result<TeardownReport, RuntimeError> {
        self.domain.stop()
    }
}
