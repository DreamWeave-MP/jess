//! `jess.gpu`: the GPU in front of the script, as `jess::gpu` reaches it. A jess built without the
//! `gpu` feature has the member, and says so when asked for a device.

use std::rc::Rc;

use l3i::Runtime;
use l3i::value::Table;

use crate::lua::plan::Functions;
use crate::lua::{Jess, guard};

/// What opening a GPU device needs: the GPU is the machine's, as its input devices are.
pub const DEVICE_CAPABILITY: &str = "gpu.device";

/// What opening a window needs: the screen in front of the person, as the GPU is.
pub const WINDOW_CAPABILITY: &str = "gpu.window";

#[cfg(feature = "gpu")]
mod device;
#[cfg(feature = "gpu")]
mod draw_set;
#[cfg(feature = "gpu")]
mod pipeline;
#[cfg(feature = "gpu")]
mod plan;
#[cfg(feature = "gpu")]
mod resources;
#[cfg(feature = "window")]
mod window;

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
    let state = jess.clone();
    module.function(
        "window",
        move |call: &l3i::bind::Call<'_>, device: l3i::stack::ValueView<'_>, desc: Option<l3i::stack::ValueView<'_>>| {
            guard(&state, "jess.gpu.window", WINDOW_CAPABILITY)?;
            #[cfg(feature = "window")]
            return window::open(call, device, desc);
            #[cfg(not(feature = "window"))]
            {
                let _ = (call, device, desc);
                Err::<(), _>(crate::lua::error(
                    "jess.gpu.window: this jess has no windows: it was built without the window feature",
                ))
            }
        },
    )?;
    module.function("pump", || {
        #[cfg(feature = "window")]
        return window::pump();
        #[cfg(not(feature = "window"))]
        Ok::<(), l3i::Error>(())
    })?;
    module.finish()
}
