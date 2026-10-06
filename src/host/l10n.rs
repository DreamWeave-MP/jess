//! The localization side of a host: the domain's registry, and the event that says it changed.
//!
//! The registry is the domain's ([`crate::host::SharedRegistry`]): `jess.l10n` under the domain reads
//! and configures the same one, and a worker whose spec shares it formats from it too. The host
//! does not intercept changes; it watches the registry's generation, which rises on every locale
//! change and every reload that swapped a bundle in, whoever made it, and broadcasts one event
//! per frame it moved in.

use std::sync::{Arc, PoisonError, RwLock};

use crate::l10n::{Generation, Registry};
use crate::runtime::{EventTarget, Message, RuntimeError, ScriptDomain};

use crate::host::SharedRegistry;

/// The event a host posts to every context when the domain's registry changed:
///
/// ```text
/// { event = 'l10n.changed', generation = <number>, locales = { 'pl-PL', 'en' } }
/// ```
///
/// `generation` is the registry's, the number `jess.l10n.generation()` returns; `locales` are
/// the preferred locales as set, canonically spelled. A context that caches formatted text
/// formats again on this event. It is posted through the domain's mailbox, so it arrives in the
/// next event drain, stamped with the domain generation at posting, and never crosses a restart.
pub const CHANGED: &str = "l10n.changed";

/// The registry and the last generation the host broadcast.
pub(crate) struct Watch {
    registry: Arc<SharedRegistry>,
    seen: Generation,
}

impl Watch {
    pub(crate) fn new(registry: Registry) -> Watch {
        let seen = registry.generation();
        Watch {
            registry: Arc::new(RwLock::new(registry)),
            seen,
        }
    }

    pub(crate) fn registry(&self) -> &Arc<SharedRegistry> {
        &self.registry
    }

    /// Posts [`CHANGED`] to every context if the generation moved since the last broadcast.
    pub(crate) fn broadcast(&mut self, domain: &ScriptDomain) -> Result<Option<Generation>, RuntimeError> {
        let registry = self.registry.read().unwrap_or_else(PoisonError::into_inner);
        if registry.generation() == self.seen {
            return Ok(None);
        }
        self.seen = registry.generation();
        let payload = changed(&registry);
        drop(registry);
        domain
            .mailbox()?
            .post(EventTarget::All, payload)
            .map_err(|closed| RuntimeError::Lifecycle {
                domain: closed.0,
                state: domain.state(),
                action: "post a localization event to",
            })?;
        Ok(Some(self.seen))
    }
}

/// The [`CHANGED`] event for the registry as it is now.
#[must_use]
pub fn changed(registry: &Registry) -> Message {
    let locales = registry
        .preferred_locales()
        .iter()
        .map(|locale| Message::text(registry.locale_name(*locale)))
        .collect();
    #[allow(clippy::cast_precision_loss)] // Generations are counted in small numbers.
    let generation = registry.generation().get() as f64;
    Message::map([
        ("event", Message::text(CHANGED)),
        ("generation", Message::Number(generation)),
        ("locales", Message::List(locales)),
    ])
}
