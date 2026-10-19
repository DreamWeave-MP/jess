//! `@jess/application`: what a script running under a host asks of the host that runs it.

use l3i::Runtime;
use l3i::value::Table;

use crate::host::ExitRequest;
use crate::lua::error;

/// `application.exit(status)`: asks the host to stop with `status` (0 without one) after the
/// current frame. Only the domain's own code asks; a worker tells the domain in a message.
fn exit(status: Option<f64>) -> l3i::Result<()> {
    let status = status.unwrap_or(0.0);
    if status.fract() != 0.0 || !(0.0..=f64::from(u8::MAX)).contains(&status) {
        return Err(error(format!(
            "application.exit: the status is an integer from 0 to 255, not {status}"
        )));
    }
    let no_host = || {
        error(
            "application.exit: needs an application host; jess run, jess test, and command targets end when their script returns",
        )
    };
    let authority = crate::runtime::current_authority().ok_or_else(no_host)?;
    if authority.attribution().worker.is_some() {
        return Err(error(
            "application.exit is for the domain's own code; a worker asks the domain in a message",
        ));
    }
    let request = authority.service::<ExitRequest>().ok_or_else(no_host)?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the guard above requires an integral value in 0..=255"
    )]
    request.request(status as u8);
    Ok(())
}

/// The `jess.application` table.
pub(crate) fn module(runtime: &Runtime) -> l3i::Result<Table> {
    let module = crate::lua::plan::Functions::module(runtime, "application")?;
    module.function("exit", exit)?;
    module.finish()
}
