//! Generic Jess services composed into an l3i runtime plan.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use l3i::Runtime;
use l3i::bind::{ArgView, Binding};
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

/// Generic extensions and explicit host grants; native compilation follows the runtime policy.
fn builder(capabilities: &[String], profiling: bool) -> RuntimePlanBuilder {
    let mut policy = RuntimePolicy::new()
        .debug_root(EXTENSION_ID)
        .profiler(profiling)
        .native_code(NativeCodePolicy {
            mode: NativeCodeMode::Eager,
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
        .extension(l3i::fs::FsExtension)
        .extension(l3i::intern::InternExtension)
        .extension(l3i::process::ProcessExtension)
        .extension(l3i::syntax::SyntaxExtension)
        .extension(crate::lua::stdlib::StdlibExtension)
        .extension(crate::runtime::luau::RuntimeExtension)
}

/// The members of `@jess`, installed per runtime. Each table among them is also the module
/// `@jess/<name>` (`require("@jess/shader")`).
pub(crate) const MEMBERS: [&str; 11] = [
    "version", "null", "fs", "toml", "regex", "shader", "context", "worker", "l10n", "input", "bench",
];

/// jess's Luau API: the `@jess` module and every jess userdata type.
#[derive(Clone, Copy, Debug, Default)]
pub struct JessExtension;

impl Extension for JessExtension {
    fn id(&self) -> &'static str {
        EXTENSION_ID
    }

    fn describe(&self, d: &mut ExtensionDescriptor) -> l3i::Result<()> {
        // `jess.context` and `jess.worker` are the runtime's `@jess/context` and `@jess/worker`.
        d.requires(crate::runtime::luau::EXTENSION_ID);
        crate::lua::regexp::describe(d);
        crate::lua::shader::describe(d);
        crate::lua::l10n::describe(d);
        crate::lua::input::describe(d);
        // `jess.fs` reaches the host's files only as far as the runtime's policy lets it.
        d.optional_capability(l3i::fs::READ_CAPABILITY);
        d.optional_capability(l3i::fs::WRITE_CAPABILITY);
        let module = d.module(MODULE);
        module.doc("Generic native services for Luau applications.");
        for name in MEMBERS {
            module.installed(name).untyped();
        }
        Ok(())
    }

    fn install(&self, cx: &mut InstallContext<'_>) -> l3i::Result<()> {
        let read = cx.has_capability(l3i::fs::READ_CAPABILITY)?;
        let write = cx.has_capability(l3i::fs::WRITE_CAPABILITY)?;
        let runtime = cx.runtime();
        let module = cx.module(MODULE)?.table().clone();
        let jess = cx.insert_state(Jess::new(runtime, module)?);
        let l10n = crate::lua::l10n::install(cx)?;
        let module = cx.module(MODULE)?;
        module.set("version", env!("CARGO_PKG_VERSION"))?;
        module.set("null", &crate::lua::convert::NULL)?;
        module.set("fs", &crate::lua::fs::module(runtime, read, write)?)?;
        module.set("bench", &crate::lua::bench::module(runtime)?)?;
        module.set("toml", &crate::lua::toml::module(runtime, &jess)?)?;
        module.set("regex", &crate::lua::regexp::module(runtime, &jess)?)?;
        module.set("shader", &crate::lua::shader::module(runtime, &jess)?)?;
        let modules = cx
            .state_of::<crate::runtime::luau::Modules>(crate::runtime::luau::EXTENSION_ID)?
            .ok_or_else(|| l3i::Error::logic("jess.runtime installed no modules"))?;
        let module = cx.module(MODULE)?;
        module.set("context", &modules.context)?;
        module.set("worker", &modules.worker)?;
        module.set("l10n", &l10n)?;
        module.set("input", &crate::lua::input::module(runtime, &jess)?)?;
        Ok(())
    }
}

impl Jess {
    fn new(runtime: &Runtime, module: Table) -> l3i::Result<Self> {
        let tostring = Function::from_value(runtime.global("tostring")?)?;
        let (array_marker, loaded) = {
            let stack = runtime.stack();
            (Table::new(&stack, 0, 0)?, Table::new(&stack, 0, 32)?)
        };
        l3i::readonly::make_read_only(runtime, &array_marker)?;
        Ok(Self {
            module,
            array_marker,
            compile_options: RefCell::new(None),
            bytecode: RefCell::new(HashMap::new()),
            loaded,
            luau_require: RefCell::new(None),
            packages: RefCell::new(None),
            tostring,
        })
    }
}

/// What `function` raises in a runtime whose policy doesn't grant `capability`.
pub(crate) fn denied(function: &str, capability: &str) -> String {
    format!("{function}: needs the '{capability}' capability, which this runtime does not grant")
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

    /// Binds `callable` as `key` when the runtime was `granted` the `capability` it needs, and
    /// otherwise a function that raises the permission error naming it, as l3i's own modules do.
    pub(crate) fn gated<F: Binding<M>, M>(
        &self,
        key: &str,
        capability: &str,
        granted: bool,
        callable: F,
    ) -> l3i::Result<&Self> {
        if granted {
            return self.function(key, callable);
        }
        let message = denied(&format!("jess.{}.{key}", self.name), capability);
        self.function(key, move |_: ArgView<'_>| -> l3i::Result<()> {
            Err(l3i::Error::permission(message.clone()))
        })
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
