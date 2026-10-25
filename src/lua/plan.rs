//! Generic Jess services composed into an l3i runtime plan.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use l3i::Runtime;
use l3i::bind::{Binding, Call};
use l3i::extension::{
    Extension, ExtensionDescriptor, InstallContext, NativeCodePolicy, RuntimePlan, RuntimePlanBuilder, RuntimePolicy,
    UserdataBuilder,
};
use l3i::native_code::NativeCodeMode;

use l3i::userdata::Userdata;
use l3i::value::{Function, Table};

use crate::lua::Jess;

/// The extension's id, and the debug root of every jess function and type.
pub const EXTENSION_ID: &str = "jess";

/// The module's `require` path; the global `jess` is the same table.
pub const MODULE: &str = "@jess";

thread_local! {
    static PLAN: RefCell<Option<Rc<RuntimePlan>>> = const { RefCell::new(None) };
    /// What each jess userdata type registers, by `Userdata::NAME`, for the contract inventory.
    static SURFACE: RefCell<BTreeMap<&'static str, Surface>> = const { RefCell::new(BTreeMap::new()) };
}

/// The names one userdata type registers.
#[derive(Clone, Debug, Default)]
pub(crate) struct Surface {
    pub(crate) metamethods: BTreeSet<String>,
}

pub(crate) fn surface_of(name: &str) -> Surface {
    SURFACE.with_borrow(|surface| surface.get(name).cloned().unwrap_or_default())
}

fn record_surface(name: &'static str, record: impl FnOnce(&mut Surface)) {
    SURFACE.with_borrow_mut(|surface| record(surface.entry(name).or_default()));
}

/// The plan every jess runtime is made from, made once per thread.
///
/// # Errors
///
/// Returns an error if the plan doesn't finalize.
pub fn plan() -> l3i::Result<Rc<RuntimePlan>> {
    if let Some(plan) = PLAN.with_borrow(Clone::clone) {
        return Ok(plan);
    }
    let plan = builder(&[], false).extension(JessExtension).finalize()?;
    PLAN.with_borrow_mut(|cached| *cached = Some(plan.clone()));
    Ok(plan)
}

/// Builds a plan granting only optional capabilities explicitly approved by a host.
///
/// # Errors
///
/// Returns an error if the runtime plan cannot be finalized.
pub fn plan_with_capabilities(capabilities: &[String]) -> l3i::Result<Rc<RuntimePlan>> {
    if capabilities.is_empty() {
        return plan();
    }
    builder(capabilities, false).extension(JessExtension).finalize()
}

/// Builds a non-cached plan with runtime call profiling and native block counters enabled.
///
/// # Errors
///
/// Returns an error if the runtime plan cannot be finalized.
pub fn plan_with_profiling(capabilities: &[String]) -> l3i::Result<Rc<RuntimePlan>> {
    builder(capabilities, true).extension(JessExtension).finalize()
}

/// The libraries' Luau type definitions, rendered from a plan of their extensions alone.
pub(crate) fn downstream_definitions() -> l3i::Result<String> {
    Ok(format!(
        "{}\n{}",
        crate::lua::stdlib::definitions(),
        builder(&[], false).finalize()?.type_definitions()
    ))
}

/// Generic extensions and explicit host grants. Native code is generated for the modules marked
/// `--!native` and no others: generating it for every chunk cost a tool run a third of a second
/// before its first record, most of it in modules the run never called.
fn builder(capabilities: &[String], profiling: bool) -> RuntimePlanBuilder {
    let mut policy = RuntimePolicy::new()
        .debug_root(EXTENSION_ID)
        .profiler(profiling)
        .native_code(NativeCodePolicy {
            mode: NativeCodeMode::Annotated,
            record_counters: profiling,
            ..NativeCodePolicy::default()
        });
    for capability in capabilities {
        policy = policy.capability(capability);
    }
    RuntimePlan::builder()
        .policy(policy)
        .extension(l3i::bytes::BytesExtension)
        .extension(l3i::data::DataExtension)
        .extension(l3i::dns::DnsExtension::default())
        .extension(l3i::fs::FsExtension)
        .extension(l3i::intern::InternExtension)
        .extension(l3i::intl::IntlExtension)
        .extension(l3i::process::ProcessExtension)
        .extension(l3i::syntax::SyntaxExtension)
        .extension(l3i::tcp::TcpExtension::default())
        .extension(l3i::tls::TlsExtension::default())
        .extension(crate::lua::stdlib::StdlibExtension)
        .extension(crate::runtime::luau::RuntimeExtension)
}

/// The members of `@jess`, installed per runtime. Each table among them is also the module
/// `@jess/<name>` (`require("@jess/shader")`).
pub(crate) const MEMBERS: [&str; 13] = [
    "version",
    "null",
    "granted",
    "payload",
    "shader",
    "context",
    "worker",
    "events",
    "interfaces",
    "application",
    "input",
    "audio",
    "bench",
];

/// The capabilities `jess.input` checks.
const INPUT_CAPABILITIES: [&str; 3] = [
    crate::input::capability::ACTIONS,
    crate::input::capability::OBSERVE,
    crate::input::capability::CONFIGURE,
];

/// jess's Luau API: the `@jess` module and every jess userdata type.
#[derive(Clone, Copy, Debug, Default)]
pub struct JessExtension;

impl Extension for JessExtension {
    fn id(&self) -> &'static str {
        EXTENSION_ID
    }

    fn describe(&self, d: &mut ExtensionDescriptor) -> l3i::Result<()> {
        // `jess.context`, `jess.worker`, and `jess.events` are the runtime's `@jess/` modules.
        d.requires(crate::runtime::luau::EXTENSION_ID);
        crate::lua::shader::describe(d);
        crate::lua::input::describe(d);
        crate::lua::audio::describe(d);
        // `jess.input` checks these against the plan outside a domain, and the broker inside one.
        for capability in INPUT_CAPABILITIES {
            d.optional_capability(capability);
        }
        // `jess.audio.output` checks this the same way.
        d.optional_capability(crate::audio::capability::OUTPUT);
        let module = d.module(MODULE);
        module.doc("Generic native services for Luau applications.");
        for name in MEMBERS {
            module.installed(name).untyped();
        }
        Ok(())
    }

    fn install(&self, cx: &mut InstallContext<'_>) -> l3i::Result<()> {
        // Every capability an extension of the plan declares, and the broker's `worker.spawn`; and
        // which of them the policy grants.
        let mut known = vec![crate::runtime::WORKER_SPAWN.to_owned()];
        for descriptor in cx.plan().descriptors() {
            for name in descriptor.capabilities().chain(descriptor.optional_capabilities()) {
                if !known.iter().any(|seen| seen == name) {
                    known.push(name.to_owned());
                }
            }
        }
        known.sort();
        let granted = known
            .iter()
            .filter(|name| cx.plan().policy().grants(name))
            .cloned()
            .collect::<Vec<_>>();
        let runtime = cx.runtime();
        let held = granted.clone();
        let asked = runtime.bind_function("jess.granted", move |_: &Call<'_>, name: String| -> l3i::Result<bool> {
            if !known.contains(&name) {
                return Err(crate::lua::error(format!(
                    "jess.granted: unknown capability '{name}'; a jess runtime can be granted {}",
                    known.join(", ")
                )));
            }
            // Inside a domain the broker holds the grant, as `guard` asks it; a question isn't a
            // denial, so nothing is reported.
            if let Some(authority) = crate::runtime::current_authority() {
                let capability = crate::runtime::CapabilityId::intern(&name).map_err(l3i::Error::from)?;
                return Ok(authority.has(capability));
            }
            Ok(held.contains(&name))
        })?;
        let module = cx.module(MODULE)?.table().clone();
        let jess = cx.insert_state(Jess::new(runtime, module, granted)?);
        // The built-in package's payload is the binary's: read from what it embeds.
        let builtin_read = runtime.bind_function(
            "jess.payload.builtin",
            |path: crate::lua::Text| -> Option<l3i::convert::NewBuffer> {
                crate::lua::builtin::file(&path.0).map(|source| l3i::convert::NewBuffer(source.as_bytes().to_vec()))
            },
        )?;
        let payload_state = Rc::clone(&jess);
        let payload = runtime.bind_function("jess.payload", move |call: &Call<'_>| {
            crate::lua::package::payload(call, &payload_state, &builtin_read)
        })?;
        let module = cx.module(MODULE)?;
        module.set("version", env!("CARGO_PKG_VERSION"))?;
        module.set("null", &crate::lua::convert::NULL)?;
        module.set("granted", &asked)?;
        module.set("payload", &payload)?;
        module.set("bench", &crate::lua::bench::module(runtime)?)?;
        module.set("shader", &crate::lua::shader::module(runtime, &jess)?)?;
        let modules = cx
            .state_of::<crate::runtime::luau::Modules>(crate::runtime::luau::EXTENSION_ID)?
            .ok_or_else(|| l3i::Error::logic("jess.runtime installed no modules"))?;
        let module = cx.module(MODULE)?;
        module.set("context", &modules.context)?;
        module.set("worker", &modules.worker)?;
        module.set("events", &modules.events)?;
        module.set("interfaces", &modules.interfaces)?;
        module.set("application", &crate::lua::application::module(runtime)?)?;
        module.set("input", &crate::lua::input::module(runtime, &jess)?)?;
        module.set("audio", &crate::lua::audio::module(runtime, &jess)?)?;
        Ok(())
    }
}

impl Jess {
    fn new(runtime: &Runtime, module: Table, granted: Vec<String>) -> l3i::Result<Self> {
        let tostring = Function::from_value(runtime.global("tostring")?)?;
        let loaded = Table::new(&runtime.stack(), 0, 32)?;
        Ok(Self {
            module,
            compile_options: RefCell::new(None),
            bytecode: RefCell::new(HashMap::new()),
            loaded,
            luau_require: RefCell::new(None),
            packages: RefCell::new(None),
            tostring,
            builtin: RefCell::new(None),
            toml_decode: RefCell::new(None),
            granted,
            outputs: crate::lua::audio::Outputs::default(),
        })
    }
}

/// A generic sub-module bound under `jess.<name>`, frozen once filled.
pub(crate) struct Functions<'r> {
    runtime: &'r Runtime,
    name: &'static str,
    table: Table,
}

impl<'r> Functions<'r> {
    /// A sub-module of `jess`, named `jess.<name>` in debug names.
    pub(crate) fn module(runtime: &'r Runtime, name: &'static str) -> l3i::Result<Self> {
        Ok(Self {
            runtime,
            name,
            table: Table::new(&runtime.stack(), 0, 8)?,
        })
    }

    pub(crate) fn function<F: Binding<M>, M>(&self, key: &str, callable: F) -> l3i::Result<&Self> {
        let function = self.runtime.bind_function(&format!("jess.{}.{key}", self.name), callable)?;
        self.table.set(&self.runtime.stack(), key, &function)?;
        Ok(self)
    }

    pub(crate) fn set<T: l3i::convert::Push + ?Sized>(&self, key: &str, value: &T) -> l3i::Result<&Self> {
        self.table.set(&self.runtime.stack(), key, value)?;
        Ok(self)
    }

    pub(crate) fn finish(self) -> l3i::Result<Table> {
        l3i::readonly::make_read_only(self.runtime, &self.table)?;
        Ok(self.table)
    }
}

/// Declares a metamethod on a jess userdata type, recording it for the contract inventory.
pub(crate) fn metamethod<T: Userdata, F: Binding<M> + Clone + 'static, M: 'static>(
    ty: &mut UserdataBuilder<'_, T>,
    name: &'static str,
    callable: F,
) {
    record_surface(T::NAME, |surface| {
        surface.metamethods.insert(name.to_owned());
    });
    ty.metamethod(name, callable);
}
