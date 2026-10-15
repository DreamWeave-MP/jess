//! `jess.l10n` (`@jess/l10n`): localized text from bundles of ICU `MessageFormat` messages.
//!
//! A script asks for a bundle once and calls it for each message: `common("file_count", { count =
//! 3 })`. Locale fallback, plural rules, and number formatting are `jess::l10n`'s; this module
//! converts the variables table and names the bundle and message in every error.
//!
//! Which registry a call reaches depends on where the script runs. Under a `ScriptDomain` that
//! carries a `RwLock<crate::l10n::Registry>` service (`jess::host` attaches one), every call reaches
//! that registry: the application's, shared by the main runtime and every worker the host shared
//! it with. Anywhere else (a plain `jess run`, a domain without the service, a worker not given
//! it) the runtime keeps a registry of its own. The API is the same either way, and a bundle
//! handle remembers the registry it came from.
//!
//! Reading text needs `l10n.read`; the functions that change what every script sees
//! (`setLocales`, `addDirectory`, `reload`) need `l10n.configure`. The checks run under a
//! domain; outside any, nothing holds authority and the calls succeed ([`crate::lua::guard`]).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, PoisonError, RwLock};

use crate::l10n::{BundleId, Diagnostic, DirectoryProvider, Locale, Registry, Value, capability};
use l3i::bind::{Call, StackResults};
use l3i::convert::FromView;
use l3i::extension::{ExtensionDescriptor, InstallContext, ModuleInstaller, TagPolicy};
use l3i::stack::{Frame, Scope, ValueView};
use l3i::userdata::{Owned, Userdata};
use l3i::value::Table;

use crate::lua::{Text, error, guard, kind, lua};

/// The module's `require` path; `jess.l10n` is the same table.
pub const MODULE: &str = "@jess/l10n";

/// The module's members, all installed per runtime.
const MEMBERS: [&str; 7] = [
    "bundle",
    "bundles",
    "locales",
    "setLocales",
    "addDirectory",
    "reload",
    "generation",
];

/// The service type a domain carries for its localization: the registry `jess::host` attaches
/// and `jess.l10n` binds to.
pub type SharedRegistry = RwLock<Registry>;

/// A runtime's own localization registry, for runs outside a domain that carries one.
pub struct State {
    own: Rc<RefCell<Registry>>,
}

/// The registry a call reaches: the domain's shared one, or the runtime's own.
#[derive(Clone)]
enum Backing {
    Own(Rc<RefCell<Registry>>),
    Shared(Arc<SharedRegistry>),
}

impl Backing {
    /// The registry for the calling script: the domain's service when it runs under a domain
    /// carrying one, the runtime's own otherwise.
    fn current(state: &State) -> Backing {
        crate::runtime::service::<SharedRegistry>().map_or_else(|| Backing::Own(Rc::clone(&state.own)), Backing::Shared)
    }

    fn read<R>(&self, body: impl FnOnce(&Registry) -> R) -> R {
        match self {
            Backing::Own(registry) => body(&registry.borrow()),
            Backing::Shared(registry) => body(&registry.read().unwrap_or_else(PoisonError::into_inner)),
        }
    }

    fn write<R>(&self, body: impl FnOnce(&mut Registry) -> R) -> R {
        match self {
            Backing::Own(registry) => body(&mut registry.borrow_mut()),
            Backing::Shared(registry) => body(&mut registry.write().unwrap_or_else(PoisonError::into_inner)),
        }
    }
}

/// A bundle handle: the registry it belongs to and the bundle's id.
#[derive(Clone)]
pub struct LuaBundle {
    registry: Backing,
    id: BundleId,
    name: String,
}

// SAFETY: plain Rust data; the destructor never touches the Lua API.
unsafe impl Userdata for LuaBundle {
    const NAME: &'static str = "jess.l10n.Bundle";
}

/// A variable's value, owned until the arguments are borrowed for formatting.
enum Variable {
    Text(String),
    Number(f64),
    Bool(bool),
}

impl Variable {
    fn value(&self) -> Value<'_> {
        match self {
            Self::Text(text) => Value::Str(text),
            Self::Number(number) => Value::Float(*number),
            Self::Bool(value) => Value::Bool(*value),
        }
    }
}

/// Installs the module and returns its table, for `jess.l10n`.
pub fn install(cx: &mut InstallContext<'_>) -> l3i::Result<Table> {
    let runtime = cx.runtime();
    let state = cx.insert_state(State {
        own: Rc::new(RefCell::new(Registry::new())),
    });
    let module = cx.module(MODULE)?;
    install_reading(runtime, &state, module)?;
    install_configuring(runtime, &state, module)?;
    Ok(module.table().clone())
}

/// The functions that read text: `l10n.read`.
fn install_reading(runtime: &l3i::Runtime, state: &Rc<State>, module: &mut ModuleInstaller<'_>) -> l3i::Result<()> {
    let bundle = {
        let state = state.clone();
        runtime.bind_function(
            "jess.l10n.bundle",
            move |name: Text, fallback: Option<Text>| -> l3i::Result<Owned<LuaBundle>> {
                guard(capability::READ)?;
                let registry = Backing::current(&state);
                let fallback = fallback.map(|fallback| parse_locale(&fallback.0)).transpose()?;
                let id = registry.write(|registry| -> l3i::Result<BundleId> {
                    let id = registry.bundle(&name.0).map_err(error)?;
                    if let Some(fallback) = fallback {
                        let locale = registry.intern_locale(&fallback).map_err(error)?;
                        registry.set_bundle_fallback(id, locale);
                    }
                    Ok(id)
                })?;
                Ok(Owned(LuaBundle {
                    registry,
                    id,
                    name: name.0,
                }))
            },
        )?
    };
    module.set("bundle", &bundle)?;

    let bundles = {
        let state = state.clone();
        runtime.bind_function("jess.l10n.bundles", move |call: &Call<'_>| -> l3i::Result<StackResults> {
            guard(capability::READ)?;
            let names: Vec<String> =
                Backing::current(&state).read(|registry| registry.bundles().map(|(_, name)| name.to_owned()).collect());
            push_strings(call, names)?;
            Ok(StackResults)
        })?
    };
    module.set("bundles", &bundles)?;

    let locales = {
        let state = state.clone();
        runtime.bind_function("jess.l10n.locales", move |call: &Call<'_>| -> l3i::Result<StackResults> {
            guard(capability::READ)?;
            let tags: Vec<String> = Backing::current(&state).read(|registry| {
                registry
                    .preferred_locales()
                    .iter()
                    .map(|locale| registry.locale_name(*locale).to_owned())
                    .collect()
            });
            push_strings(call, tags)?;
            Ok(StackResults)
        })?
    };
    module.set("locales", &locales)?;

    let state = state.clone();
    let generation = runtime.bind_function("jess.l10n.generation", move || -> l3i::Result<f64> {
        guard(capability::READ)?;
        Ok(generation_number(Backing::current(&state).read(Registry::generation)))
    })?;
    module.set("generation", &generation)?;
    Ok(())
}

/// The functions that change what every script sees: `l10n.configure`.
fn install_configuring(runtime: &l3i::Runtime, state: &Rc<State>, module: &mut ModuleInstaller<'_>) -> l3i::Result<()> {
    let set_locales = {
        let state = state.clone();
        runtime.bind_function(
            "jess.l10n.setLocales",
            move |call: &Call<'_>, locales: ValueView<'_>| -> l3i::Result<f64> {
                guard(capability::CONFIGURE)?;
                let tags = if locales.is_string() {
                    vec![Text::from_view(locales)?.0]
                } else {
                    call.with_frame(|frame| lua::string_list(frame, locales, "locales"))?
                };
                let locales = tags
                    .iter()
                    .map(|tag| parse_locale(tag))
                    .collect::<l3i::Result<Vec<Locale>>>()?;
                let generation = Backing::current(&state)
                    .write(|registry| registry.set_preferred_locales(&locales))
                    .map_err(error)?;
                Ok(generation_number(generation))
            },
        )?
    };
    module.set("setLocales", &set_locales)?;

    let add_directory = {
        let state = state.clone();
        runtime.bind_function(
            "jess.l10n.addDirectory",
            move |path: Text, name: Option<Text>| -> l3i::Result<()> {
                guard(capability::CONFIGURE)?;
                let name = name.map_or_else(|| path.0.clone(), |name| name.0);
                Backing::current(&state).write(|registry| {
                    registry.add_provider(DirectoryProvider::new(name, path.0));
                });
                Ok(())
            },
        )?
    };
    module.set("addDirectory", &add_directory)?;

    let reload = {
        let state = state.clone();
        runtime.bind_function("jess.l10n.reload", move |call: &Call<'_>| -> l3i::Result<StackResults> {
            guard(capability::CONFIGURE)?;
            let report = Backing::current(&state).write(Registry::reload);
            lua::push_list(call, report.diagnostics.iter(), push_diagnostic)?;
            Ok(StackResults)
        })?
    };
    module.set("reload", &reload)?;
    Ok(())
}

/// Declares the module and the bundle type.
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut bundle = d.userdata::<LuaBundle>(LuaBundle::NAME);
    bundle
        .tag(TagPolicy::Never)
        .doc("A bundle of localized messages; call it with a message id and its variables.");
    bundle.getter("name", |this: &LuaBundle| this.name.clone()).untyped();
    bundle
        .method(
            "format",
            |this: &LuaBundle, call: &Call<'_>, id: Text, variables: Option<ValueView<'_>>| -> l3i::Result<String> {
                format(this, call, &id.0, variables)
            },
        )
        .untyped();
    bundle
        .method("has", |this: &LuaBundle, id: Text| -> l3i::Result<bool> {
            guard(capability::READ)?;
            Ok(this.registry.read(|registry| {
                registry
                    .message_id(this.id, &id.0)
                    .is_some_and(|message| registry.lookup(message).is_some())
            }))
        })
        .untyped();
    crate::lua::plan::metamethod(
        &mut bundle,
        "__call",
        |this: &LuaBundle, call: &Call<'_>, id: Text, variables: Option<ValueView<'_>>| -> l3i::Result<String> {
            format(this, call, &id.0, variables)
        },
    );
    crate::lua::plan::metamethod(&mut bundle, "__tostring", |this: &LuaBundle| {
        format!("L10nBundle({})", this.name)
    });

    let module = d.module(MODULE);
    module.doc("Localization: bundles of ICU MessageFormat messages, resolved through the locale chain.");
    for name in MEMBERS {
        module.installed(name).untyped();
    }
}

/// Formats one message of a bundle with the variables a script passed.
fn format(this: &LuaBundle, call: &Call<'_>, id: &str, variables: Option<ValueView<'_>>) -> l3i::Result<String> {
    guard(capability::READ)?;
    let variables = call.with_frame(|frame| read_variables(frame, this, id, variables))?;
    let args: Vec<(&str, Value<'_>)> = variables
        .iter()
        .map(|(name, variable)| (name.as_str(), variable.value()))
        .collect();
    this.registry.write(|registry| {
        let message = registry.message(this.id, id);
        registry.format_to_string(message, &args).map_err(error)
    })
}

/// The variables table as owned values: string keys, and string, number, or boolean values.
fn read_variables(
    frame: &Frame<'_>,
    this: &LuaBundle,
    id: &str,
    variables: Option<ValueView<'_>>,
) -> l3i::Result<Vec<(String, Variable)>> {
    let Some(variables) = variables.filter(|variables| !variables.is_nil()) else {
        return Ok(Vec::new());
    };
    if !variables.is_table() {
        return Err(error(format!(
            "variables for message '{id}' of bundle '{}' must be a table, not {}",
            this.name,
            kind(variables)
        )));
    }
    let mut owned = Vec::new();
    variables.as_table()?.for_each(frame, |_, key, value| {
        if !key.is_string() {
            return Err(error(format!(
                "variables for message '{id}' of bundle '{}' are named by strings, not {}",
                this.name,
                kind(key)
            )));
        }
        let name = key.read::<&str>()?.to_owned();
        let variable = if value.is_string() {
            Variable::Text(value.read::<&str>()?.to_owned())
        } else if value.is_boolean() {
            Variable::Bool(value.read::<bool>()?)
        } else if value.is_number() || value.is_integer() {
            Variable::Number(value.read::<f64>()?)
        } else {
            return Err(error(format!(
                "variable '{name}' of message '{id}' in bundle '{}' must be a string, number, or boolean, not {}",
                this.name,
                kind(value)
            )));
        };
        owned.push((name, variable));
        Ok(())
    })?;
    Ok(owned)
}

fn parse_locale(tag: &str) -> l3i::Result<Locale> {
    Locale::parse(tag).map_err(error)
}

/// A generation as a Luau number; generations are counts of changes, far below 2^53.
#[allow(clippy::cast_precision_loss)]
fn generation_number(generation: crate::l10n::Generation) -> f64 {
    generation.get() as f64
}

fn push_strings(scope: &impl Scope, strings: Vec<String>) -> l3i::Result<()> {
    lua::push_list(scope, strings.into_iter(), |frame, text| {
        frame.push(text.as_str())?;
        Ok(())
    })
}

/// A diagnostic as a table: `severity`, `kind`, `detail`, `generation`, and whichever of
/// `provider`, `path`, `line`, `column`, `bundle`, `locale`, and `message` it has.
fn push_diagnostic(frame: &Frame<'_>, diagnostic: &Diagnostic) -> l3i::Result<()> {
    lua::push_table(frame, 0, 11, |frame, table| {
        let text = |frame: &Frame<'_>, key: &str, value: &str| -> l3i::Result<()> {
            frame.push(value)?;
            table.raw_set(frame, key)
        };
        text(frame, "severity", diagnostic.severity.as_str())?;
        text(frame, "kind", diagnostic.kind.code())?;
        text(frame, "detail", &diagnostic.detail)?;
        frame.push_number(generation_number(diagnostic.generation))?;
        table.raw_set(frame, "generation")?;
        for (key, value) in [
            ("provider", &diagnostic.provider),
            ("path", &diagnostic.path),
            ("bundle", &diagnostic.bundle),
            ("locale", &diagnostic.locale),
            ("message", &diagnostic.message),
        ] {
            if let Some(value) = value {
                text(frame, key, value)?;
            }
        }
        for (key, value) in [("line", diagnostic.line), ("column", diagnostic.column)] {
            if let Some(value) = value {
                frame.push_number(f64::from(value))?;
                table.raw_set(frame, key)?;
            }
        }
        Ok(())
    })
}
