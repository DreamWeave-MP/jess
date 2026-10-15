//! The opaque identities `jess::input` owns.
//!
//! Every id is a plain integer allocated by its owning registry, never a pointer: a device id by
//! the device registry behind a queue, action, set, layer, binding, and node ids by the compiled
//! graph that declared them. Ids from one owner mean nothing to another; the `Display` forms are for diagnostics, not for persistence.

use std::fmt;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident, $label:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
        #[derive(serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(u32);

        impl $name {
            /// The id as the integer its owner allocated.
            #[must_use]
            pub const fn index(self) -> u32 {
                self.0
            }

            pub(crate) fn from_position(position: usize) -> Self {
                Self(u32::try_from(position).expect("jess input registries hold fewer than 2^32 entries"))
            }

            pub(crate) fn position(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}#{}", $label, self.0)
            }
        }
    };
}

id_type! {
    /// A connected device's identity, allocated by an [`crate::input::InputQueue`]'s device registry.
    ///
    /// The id lives for the queue's lifetime. A device that disconnects and reconnects with the
    /// same backend identity ([`crate::input::DeviceIdentity`]) gets its id back; one the platform can't
    /// tell from a new device gets a new id. Ids are never reused for a different identity.
    DeviceId, "device"
}

id_type! {
    /// A semantic action, allocated by the [`crate::input::ActionGraph`] that declares it.
    ActionId, "action"
}

id_type! {
    /// An action set, allocated by the [`crate::input::ActionGraph`] that declares it.
    SetId, "set"
}

id_type! {
    /// An action layer, allocated by the [`crate::input::ActionGraph`] that declares it.
    LayerId, "layer"
}

id_type! {
    /// A node of a compiled action graph, allocated by the [`crate::input::ActionGraph`] that holds it.
    NodeId, "node"
}

id_type! {
    /// A compiled binding, allocated by the [`crate::input::ActionGraph`] that holds it: the answer to
    /// "why did this action fire".
    BindingId, "binding"
}

/// A monotonic generation counter, bumped by its owner every time it replaces what it holds.
///
/// The input runtime bumps its generation on every graph install and uninstall, and everything
/// a graph produces (its action events) carries the generation it was produced
/// under, so a consumer can drop what an earlier graph produced instead of acting on it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Generation(u64);

impl Generation {
    /// The generation before anything was installed.
    pub const INITIAL: Self = Self(0);

    /// The generation as its integer.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// The generation after this one.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for Generation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "generation {}", self.0)
    }
}
