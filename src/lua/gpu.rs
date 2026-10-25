//! `jess.gpu`: the GPU in front of the script, as `jess::gpu` reaches it. A jess built without the
//! `gpu` feature has the member, and says so when asked for a device.

use std::rc::Rc;

use l3i::Runtime;
use l3i::value::Table;

use crate::lua::plan::Functions;
use crate::lua::{Jess, guard};

/// What opening a GPU device needs: the GPU is the machine's, as its input devices are.
pub const DEVICE_CAPABILITY: &str = "gpu.device";

#[cfg(feature = "gpu")]
mod device;
#[cfg(feature = "gpu")]
mod draw_set;
#[cfg(feature = "gpu")]
mod pipeline;
#[cfg(feature = "gpu")]
mod resources;

#[cfg(feature = "gpu")]
pub use device::describe;

/// The `jess.gpu` table.
pub fn module(runtime: &Runtime, jess: &Rc<Jess>) -> l3i::Result<Table> {
    let module = Functions::module(runtime, "gpu")?;
    let state = jess.clone();
    #[cfg(feature = "gpu")]
    let opened = device::Opened::default();
    module.function("device", move || {
        guard(&state, "jess.gpu.device", DEVICE_CAPABILITY)?;
        #[cfg(feature = "gpu")]
        return opened.device();
        #[cfg(not(feature = "gpu"))]
        Err::<(), _>(crate::lua::error(
            "jess.gpu.device: this jess has no GPU: it was built without the gpu feature",
        ))
    })?;
    module.finish()
}
