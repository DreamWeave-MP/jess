//! What a domain adds to l3i's stdin: under a dispatch the check is the running code's authority,
//! as at every other native boundary, and nothing frame-driven may block on a terminal.
//!
//! Reads are not checked. The handle is the authority to read, so handing one to a less privileged
//! context delegates that. Runtime teardown releases the claim through the handle's destructor;
//! the reader thread stays in l3i, never joined and never calling Luau.

use l3i::process::STDIO_CAPABILITY;
use l3i::process::input::{Operation, Policy};

use crate::runtime::capability::current_authority;
use crate::runtime::ids::CapabilityId;

/// The stdin policy of every jess runtime. Outside any dispatch (`jess run`, a command target)
/// there is no scheduler to stall and the runtime's own grant has already decided.
pub struct StdinPolicy;

impl Policy for StdinPolicy {
    fn check(&self, operation: Operation) -> l3i::Result<()> {
        let Some(authority) = current_authority() else {
            return Ok(());
        };
        match operation {
            Operation::Open => {
                let stdio = CapabilityId::intern(STDIO_CAPABILITY).map_err(l3i::Error::from)?;
                authority
                    .require(stdio)
                    .map_err(|denied| l3i::Error::from(crate::runtime::RuntimeError::CapabilityDenied(denied)))
            }
            Operation::Wait => Err(l3i::Error::permission(
                "dream.process.stdin.wait: would block this domain's scheduler; call read or readLine on a later frame and retry on wouldBlock",
            )),
        }
    }
}
