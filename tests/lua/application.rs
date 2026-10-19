//! Application targets: managed modules in one domain, their host callbacks and named events, and
//! the domain's interfaces with their overrides, from Package Sources and from JPKs.
//!
//! The example under `tests/lua/application/` is four packages: `org.example.journal` (an export
//! the others write what they did to), `org.example.inventory` (provides `game.inventory`),
//! `org.example.armor` (depends on the inventory and replaces its interface, keeping its weights),
//! and `org.example.game` (the application target, with a director and a scoreboard). Tests that
//! need something the example doesn't show write small packages of their own beside it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use jess::lua::Options;
use jess::lua::application::Application;
use jess::lua::l3i;
use jess::lua::package::{self, Launched, TargetRequest};
use jess::runtime::{DiagnosticKind, Message, Phase};

use super::write;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lua/application")
}

fn text(path: &Path) -> String {
    path.display().to_string()
}

/// A manifest for `id` 1.0.0 whose payload is `src`, with `body` after its `[package]` table.
fn manifest(id: &str, body: &str) -> String {
    format!(
        "manifest_version = 1\n\n[package]\nid = \"{id}\"\nversion = \"1.0.0\"\njess_api = \"1\"\ninclude = [\"src\"]\n\n{body}"
    )
}

/// `[dependencies]` lines requiring `^1.0` of each of `ids`.
fn requirements(ids: &[&str]) -> String {
    ids.iter()
        .map(|id| format!("\"{id}\" = \"^1.0\"\n"))
        .collect::<Vec<_>>()
        .concat()
}

/// The request for `target` of the package at `package`, its dependencies found among the example's
/// packages and in `paths`, granted nothing beyond what running it grants.
fn request(package: &Path, target: &str, paths: &[&Path]) -> TargetRequest {
    let mut paths = paths.iter().map(|path| text(path)).collect::<Vec<_>>();
    paths.push(text(&fixtures()));
    TargetRequest {
        package: text(package),
        target: Some(target.to_owned()),
        paths,
        grant: Some(Vec::new()),
        ..TargetRequest::default()
    }
}

fn try_launch(request: &TargetRequest) -> l3i::Result<Application> {
    match package::launch(&Options::default(), request)? {
        Launched::Application(application) => Ok(application),
        Launched::Command(_) => panic!("{} is a command target", request.package),
    }
}

fn launch(request: &TargetRequest) -> Application {
    try_launch(request).unwrap_or_else(|error| panic!("{error}"))
}

fn launch_error(request: &TargetRequest) -> String {
    try_launch(request).expect_err("the launch fails").to_string()
}

/// What the example's packages wrote to the journal so far, read at host level.
fn journal(application: &Application) -> Vec<String> {
    application
        .domain()
        .with_runtime(|runtime| runtime.eval::<String>("return table.concat(require('@example/journal').lines(), '\\n')"))
        .expect("the application has a runtime")
        .unwrap_or_else(|error| panic!("{error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Evaluates `source` in the application's runtime at host level.
fn eval(application: &Application, source: &str) -> String {
    application
        .domain()
        .with_runtime(|runtime| runtime.eval::<String>(source))
        .expect("the application has a runtime")
        .unwrap_or_else(|error| panic!("{error}"))
}

/// The error `source` raises in the application's runtime at host level.
fn refusal(application: &Application, source: &str) -> String {
    application
        .domain()
        .with_runtime(|runtime| runtime.eval::<()>(source))
        .expect("the application has a runtime")
        .expect_err("the script raises")
        .to_string()
}

fn frame(application: &mut Application) -> jess::host::FrameReport {
    let report = application.frame(Duration::from_millis(16)).unwrap();
    assert_eq!(report.failures(), Vec::<&jess::runtime::RuntimeError>::new());
    report
}

/// The journal of the example, start to the frame it asks to stop on.
const EXAMPLE: [&str; 13] = [
    "armor replaces light armor, keeping its weights",
    "entry: heavy helm, weighs 5",
    "director load",
    "inventory startup",
    "director startup: heavy cuirass, weighs 30",
    "director update 1 (number)",
    "director update 2 (number)",
    "inventory drops Caius's loot",
    "armor dents for Caius",
    "scoreboard: death 1, Caius on frame 2",
    "director update 3 (number)",
    "director game over: Caius died",
    "director update 4 (number)",
];

/// Starts the example and runs it until it asks to stop, returning its journal and the frames run.
fn run_example(application: &mut Application) -> (Vec<String>, usize) {
    let started = application.start().unwrap();
    assert!(started.failures.is_empty(), "{:?}", started.failures);
    let mut frames = 0;
    while application.exit_requested().is_none() {
        frame(application);
        frames += 1;
        assert!(frames < 100, "the example asks to stop on its fourth frame");
    }
    (journal(application), frames)
}

#[test]
fn an_application_target_loads_its_managed_modules_into_one_domain() {
    let mut application = launch(&request(&fixtures().join("game"), "game", &[]));

    // Loading ran the managed modules, composed the interface (armor's override ran once, the
    // inventory's first provision ran none), ran the entry, then the load handlers.
    assert_eq!(journal(&application), EXAMPLE[..3]);
    let contexts = application
        .domain()
        .contexts()
        .into_iter()
        .map(|context| (context.name, context.schedule, context.events))
        .collect::<Vec<_>>();
    let names = |events: &[&str]| events.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert_eq!(
        contexts,
        [
            (
                "org.example.inventory/inventory".to_owned(),
                vec![Phase::Startup, Phase::Event],
                names(&["playerDied"])
            ),
            (
                "org.example.armor/armor".to_owned(),
                vec![Phase::Event],
                names(&["playerDied"])
            ),
            (
                "org.example.game/director".to_owned(),
                vec![Phase::Load, Phase::Startup, Phase::Update, Phase::Event, Phase::Shutdown],
                names(&["gameOver"])
            ),
            (
                "org.example.game/scoreboard".to_owned(),
                vec![Phase::Event],
                names(&["playerDied"])
            ),
        ]
    );

    // Startup, frames, named events in subscriber order, the nested emit a frame later, and the
    // exit the director asks for on its fourth update.
    let (lines, frames) = run_example(&mut application);
    assert_eq!(lines, EXAMPLE[..13]);
    assert_eq!((frames, application.exit_requested()), (4, Some(0)));
    assert_eq!(application.diagnostics(), Vec::<String>::new());

    // The entry's `return 7` changed nothing, and teardown runs the shutdown handlers.
    let teardown = application.stop().unwrap();
    assert_eq!(teardown.shutdown_failures, []);
    assert_eq!(teardown.contexts_removed, 4);
}

#[test]
fn a_package_source_and_its_jpks_run_the_same_application() {
    let temp = tempfile::tempdir().unwrap();
    let packed = temp.path().join("packages");
    std::fs::create_dir_all(&packed).unwrap();
    let runtime = jess::lua::runtime(Options::default()).unwrap();
    let mut digests = Vec::new();
    for name in ["journal", "inventory", "armor", "game"] {
        let jpk = packed.join(format!("{name}.jpk"));
        let written = package::pack(&runtime, &text(&fixtures().join(name)), &text(&jpk)).unwrap();
        digests.push(written.digest);
    }
    let from_jpks = TargetRequest {
        package: text(&packed.join("game.jpk")),
        target: Some("game".to_owned()),
        paths: vec![text(&packed)],
        grant: Some(Vec::new()),
        ..TargetRequest::default()
    };
    let mut source = launch(&request(&fixtures().join("game"), "game", &[]));
    let mut jpk = launch(&from_jpks);
    let (source_lines, _) = run_example(&mut source);
    let (jpk_lines, _) = run_example(&mut jpk);
    assert_eq!(source_lines, jpk_lines);
    assert_eq!(jpk_lines, EXAMPLE[..13]);
    // A packed module's chunk is named by its package, never by a path on disk.
    let source_of = eval(
        &jpk,
        "return debug.info(require('@jess/interfaces').get('game.inventory').getArmorType, 's')",
    );
    assert_eq!(source_of, "org.example.armor@1.0.0/src/armor.jsl");
    for (name, digest) in ["journal", "inventory", "armor", "game"].iter().zip(&digests) {
        let opened = package::inspect(&runtime, &text(&fixtures().join(name))).unwrap();
        assert_eq!(&opened.digest, digest, "{name}");
    }
    source.stop().unwrap();
    jpk.stop().unwrap();
}

/// A provider without `interfaceOverride` replaces the table outright: nothing is merged.
#[test]
fn an_override_without_a_callback_replaces_the_whole_table() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("bare");
    write(
        &root,
        &[
            (
                "jess.toml",
                &manifest(
                    "org.example.bare",
                    "[dependencies]\n\"org.example.inventory\" = \"^1.0\"\n\n[managed.bare]\nmodule = \"src/bare.jsl\"\ninterface = \"game.inventory\"\n\n[targets.app]\nkind = \"application\"\nentry = \"src/main.jsl\"\n",
                ),
            ),
            (
                "src/bare.jsl",
                "return { interface = { getArmorType = function() return 'bare' end } }\n",
            ),
            ("src/main.jsl", "return nil\n"),
        ],
    );
    let application = launch(&request(&root, "app", &[]));
    let seen = eval(
        &application,
        r"
        local inventory = require('@jess/interfaces').get('game.inventory')
        return `{inventory.getArmorType('helm')} {tostring(inventory.getWeight)}`
        ",
    );
    assert_eq!(seen, "bare nil");
    // A composed interface is frozen, like a package export.
    let patched = refusal(
        &application,
        "require('@jess/interfaces').get('game.inventory').getWeight = function() return 0 end",
    );
    assert!(patched.contains("readonly"), "{patched}");
    application.stop().unwrap();
}

/// Writes a package `id` depending on `dependencies` that provides `game.inventory` from one
/// managed module, whose override callback journals what it was handed and wraps `getWeight`.
fn wrapper(root: &Path, id: &str, dependencies: &[&str], name: &str) {
    let dependencies = requirements(dependencies);
    write(
        root,
        &[
            (
                "jess.toml",
                &manifest(
                    id,
                    &format!(
                        "[dependencies]\n\"org.example.journal\" = \"^1.0\"\n{dependencies}\n[managed.wrap]\nmodule = \"src/wrap.jsl\"\ninterface = \"game.inventory\"\n"
                    ),
                ),
            ),
            (
                "src/wrap.jsl",
                &format!(
                    r"local journal = require('@example/journal')
local mine = {{ getArmorType = function() return '{name}' end }}

return {{
  on = {{
    host = {{
      interfaceOverride = function(previous)
        journal.write(`{name} replaces {{previous.getArmorType('helm')}}`)
        mine.getWeight = function(item) return previous.getWeight(item) + 1 end
      end,
    }},
  }},
  interface = mine,
}}
"
                ),
            ),
        ],
    );
}

/// A root application depending on `dependencies`, whose entry journals the effective inventory.
fn root(root: &Path, dependencies: &[&str]) {
    let dependencies = requirements(dependencies);
    write(
        root,
        &[
            (
                "jess.toml",
                &manifest(
                    "org.example.root",
                    &format!(
                        "[dependencies]\n\"org.example.journal\" = \"^1.0\"\n{dependencies}\n[targets.app]\nkind = \"application\"\nentry = \"src/main.jsl\"\n"
                    ),
                ),
            ),
            (
                "src/main.jsl",
                r"local inventory = require('@jess/interfaces').get('game.inventory')
require('@example/journal').write(`effective {inventory.getArmorType('helm')} {inventory.getWeight('helm')}`)
",
            ),
        ],
    );
}

#[test]
fn overrides_chain_in_dependency_order_and_each_replacing_provider_is_called_once() {
    let temp = tempfile::tempdir().unwrap();
    // C depends on B (armor), which depends on A (inventory).
    wrapper(
        &temp.path().join("enchant"),
        "org.example.enchant",
        &["org.example.armor"],
        "enchanted",
    );
    root(&temp.path().join("root"), &["org.example.enchant"]);
    let application = launch(&request(&temp.path().join("root"), "app", &[temp.path()]));
    assert_eq!(
        journal(&application),
        [
            "armor replaces light armor, keeping its weights",
            "enchanted replaces heavy",
            "effective enchanted 6",
        ]
    );
    application.stop().unwrap();
}

#[test]
fn unrelated_providers_override_by_package_id_whatever_order_they_are_offered_in() {
    let temp = tempfile::tempdir().unwrap();
    // Neither depends on the other, so package ids order them: zeta after alpha.
    wrapper(
        &temp.path().join("zeta"),
        "org.example.zeta",
        &["org.example.inventory"],
        "zeta",
    );
    wrapper(
        &temp.path().join("alpha"),
        "org.example.alpha",
        &["org.example.inventory"],
        "alpha",
    );
    root(&temp.path().join("root"), &["org.example.zeta", "org.example.alpha"]);
    let expected = ["alpha replaces light", "zeta replaces alpha", "effective zeta 7"];
    for offered in [["zeta", "alpha"], ["alpha", "zeta"]] {
        let mut request = request(&temp.path().join("root"), "app", &[]);
        request.packages = offered.iter().map(|name| text(&temp.path().join(name))).collect();
        let application = launch(&request);
        assert_eq!(journal(&application), expected, "offered {offered:?}");
        application.stop().unwrap();
    }
}

/// A one-package application whose managed modules are `managed` (id, source, interface) and
/// whose entry is `entry`, with `[targets.app]` declaring `capabilities`.
fn single(root: &Path, managed: &[(&str, &str, Option<&str>)], entry: &str, capabilities: &[&str]) {
    let mut sections = vec!["[dependencies]\n\"org.example.journal\" = \"^1.0\"\n".to_owned()];
    let mut files = vec![("src/main.jsl".to_owned(), entry.to_owned())];
    for (id, source, interface) in managed {
        let interface = interface.map_or_else(String::new, |interface| format!("interface = \"{interface}\"\n"));
        sections.push(format!("[managed.{id}]\nmodule = \"src/{id}.jsl\"\n{interface}"));
        files.push((format!("src/{id}.jsl"), (*source).to_owned()));
    }
    let capabilities = capabilities
        .iter()
        .map(|capability| format!("\"{capability}\""))
        .collect::<Vec<_>>()
        .join(", ");
    sections.push(format!(
        "[targets.app]\nkind = \"application\"\nentry = \"src/main.jsl\"\ncapabilities = [{capabilities}]\n"
    ));
    let manifest = manifest("org.example.single", &sections.join("\n"));
    let mut all = vec![("jess.toml", manifest.as_str())];
    all.extend(files.iter().map(|(path, source)| (path.as_str(), source.as_str())));
    write(root, &all);
}

#[test]
fn named_events_reach_subscribers_in_order_and_one_failure_stops_none() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("events");
    single(
        &root,
        &[
            (
                "first",
                "local journal = require('@example/journal')\nreturn { on = { event = { hit = function(payload) journal.write(`first {payload.n}`) end, ['game.playerDied'] = function() journal.write('first died') end } } }\n",
                None,
            ),
            (
                "second",
                "return { on = { event = { hit = function(payload) error(`second can't take {payload.n}`) end } } }\n",
                None,
            ),
            (
                "third",
                "local journal = require('@example/journal')\nreturn { on = { event = { hit = function(payload) journal.write(`third {payload.n}`) end, other = function() journal.write('other') end } } }\n",
                None,
            ),
        ],
        "return nil\n",
        &[],
    );
    let mut application = launch(&request(&root, "app", &[]));
    application.start().unwrap();
    eval(
        &application,
        r"
        local events = require('@jess/events')
        events.emit('hit', { n = 1 })
        events.emit('game.playerDied')
        events.emit('nobody')
        events.emit('hit', { n = 2 })
        return ''
        ",
    );
    let report = application.frame(Duration::from_millis(16)).unwrap();
    assert_eq!(
        (
            report.tick.events.delivered,
            report.tick.events.unhandled,
            report.failures().len()
        ),
        (7, 1, 2)
    );
    assert_eq!(
        journal(&application),
        ["first 1", "third 1", "first died", "first 2", "third 2"]
    );
    let diagnostics = application.diagnostics();
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert!(
        diagnostics[0].starts_with(
            "error in org.example.single/second during event 'hit' (org.example.single@1.0.0/src/second.jsl:1): "
        ),
        "{}",
        diagnostics[0]
    );
    assert!(diagnostics[0].contains("second can't take 1"), "{}", diagnostics[0]);
    application.stop().unwrap();
}

#[test]
fn script_events_never_reach_host_callbacks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("privilege");
    single(
        &root,
        &[(
            "watch",
            r"local journal = require('@example/journal')
local mine = {}
return {
  on = {
    host = {
      startup = function() journal.write('host startup') end,
      update = function() journal.write('host update') end,
      shutdown = function() journal.write('host shutdown') end,
      interfaceOverride = function() journal.write('host interfaceOverride') end,
    },
  },
  interface = mine,
}
",
            Some("game.watch"),
        )],
        "return nil\n",
        &[],
    );
    let mut application = launch(&request(&root, "app", &[]));
    application.start().unwrap();
    eval(
        &application,
        r"
        local events = require('@jess/events')
        for _, name in { 'startup', 'update', 'shutdown', 'interfaceOverride', 'load', 'event' } do
          events.emit(name, {})
        end
        return ''
        ",
    );
    let report = frame(&mut application);
    assert_eq!((report.tick.events.delivered, report.tick.events.unhandled), (0, 6));
    // Only the one real update ran; the first provider's override never did.
    assert_eq!(journal(&application), ["host startup", "host update"]);

    // A script can't make a host callback a phase, or bind to a phase its context lacks.
    let replaced = refusal(
        &application,
        "require('@jess/context').bind('org.example.single/watch', { interfaceOverride = function() end })",
    );
    assert!(
        replaced.contains("'interfaceOverride' is not a scheduler phase"),
        "{replaced}"
    );
    let unscheduled = refusal(
        &application,
        "require('@jess/context').bind('org.example.single/watch', { input = function() end })",
    );
    assert!(unscheduled.contains("does not schedule the input phase"), "{unscheduled}");
    application.stop().unwrap();
}

#[test]
fn two_domains_have_their_own_interfaces_and_share_events_only_when_routed() {
    let temp = tempfile::tempdir().unwrap();
    // The second application selects the inventory alone: light armor, where the game's is heavy.
    root(&temp.path().join("light"), &["org.example.inventory"]);
    let mut game = launch(&request(&fixtures().join("game"), "game", &[]));
    let mut light = launch(&request(&temp.path().join("light"), "app", &[]));
    let armor = "return require('@jess/interfaces').get('game.inventory').getArmorType('helm')";
    assert_eq!(
        (eval(&game, armor), eval(&light, armor)),
        ("heavy".to_owned(), "light".to_owned())
    );
    assert_eq!(journal(&light), ["effective light 5"], "each domain has its own journal too");

    game.start().unwrap();
    light.start().unwrap();
    // The game's death stays in the game until its host routes the event.
    frame(&mut game);
    frame(&mut game);
    frame(&mut light);
    assert_eq!(journal(&light), ["effective light 5", "inventory startup"]);
    game.domain().route("playerDied", light.domain().mailbox().unwrap()).unwrap();
    eval(
        &game,
        "require('@jess/events').emit('playerDied', { player = 'Fargoth', frame = 9 }) return ''",
    );
    frame(&mut light);
    assert_eq!(
        journal(&light),
        ["effective light 5", "inventory startup", "inventory drops Fargoth's loot"]
    );
    // Only transferable values cross, and a refusal names the path.
    let refused = game
        .domain()
        .with_runtime(|runtime| {
            runtime.eval::<()>("require('@jess/events').emit('playerDied', { player = 'x', onDeath = function() end })")
        })
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(refused.contains("payload.onDeath: a function can't cross"), "{refused}");
    assert_eq!(
        game.drain_diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.kind == DiagnosticKind::TransferUnsupported)
            .count(),
        1
    );
    game.stop().unwrap();
    light.stop().unwrap();
}

#[test]
fn a_restart_rebuilds_modules_handlers_and_interfaces_and_drops_old_events() {
    let mut application = launch(&request(&fixtures().join("game"), "game", &[]));
    run_example(&mut application);
    let generation = application.domain().generation();
    application
        .domain()
        .mailbox()
        .unwrap()
        .post_named(
            "playerDied",
            Message::map([("player", Message::text("stale")), ("frame", Message::Number(0.0))]),
        )
        .unwrap();
    let restarted = application.restart().unwrap();
    assert_eq!(restarted, generation.next());
    assert_eq!(application.exit_requested(), None);
    // A new runtime: the journal, the module tables, and the interfaces are all new, and the
    // override ran again against the new inventory.
    assert_eq!(
        journal(&application),
        [EXAMPLE[0], EXAMPLE[1], EXAMPLE[2], EXAMPLE[3], EXAMPLE[4]]
    );
    assert_eq!(application.domain().contexts().len(), 4, "the same contexts, declared again");
    let report = frame(&mut application);
    assert_eq!((report.tick.events.stale, report.tick.events.delivered), (1, 0));
    assert_eq!(journal(&application).last().unwrap(), "director update 1 (number)");
    let teardown = application.stop().unwrap();
    assert_eq!((teardown.workers_stopped, teardown.resources_released), (0, 0));
}

/// Launches a one-module application whose module is `source` and returns the launch error.
fn refused(source: &str, interface: Option<&str>) -> String {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("bad");
    single(&root, &[("broken", source, interface)], "return nil\n", &[]);
    launch_error(&request(&root, "app", &[]))
}

#[test]
fn a_malformed_managed_module_fails_the_launch_naming_package_module_and_key() {
    let at = "org.example.single target app failed to load:\nerror in org.example.single:app during load: managed 'broken' of org.example.single (org.example.single@1.0.0/src/broken.jsl): ";
    for (source, interface, problem) in [
        ("return 5", None, "returned a number, not a table of on and interface"),
        ("return { handlers = {} }", None, "returned an unknown key 'handlers'"),
        (
            "return { on = { hosts = {} } }",
            None,
            "on.hosts is not a handler group; expected host or event",
        ),
        (
            "return { on = { host = { updat = function() end } } }",
            None,
            "on.host.updat is not a host callback; expected load, startup, fixedUpdate, update",
        ),
        (
            "return { on = { host = { event = function() end } } }",
            None,
            "on.host.event is not a host callback; raw events are @jess/context's",
        ),
        (
            "return { on = { host = { timer = function() end } } }",
            None,
            "on.host.timer is not a host callback; nothing in the kernel dispatches the timer phase yet",
        ),
        (
            "return { on = { host = { update = 5 } } }",
            None,
            "on.host.update is a number, not a function",
        ),
        (
            "return { on = { event = { ['game.hit'] = 'nope' } } }",
            None,
            "on.event[\"game.hit\"] is a string, not a function",
        ),
        (
            "return { on = { event = { [1] = function() end } } }",
            None,
            "on.event is keyed by names, and has a number key",
        ),
        (
            "return {}",
            Some("game.broken"),
            "[managed.broken] provides the interface 'game.broken', and the module returned no interface table",
        ),
        (
            "return { interface = {} }",
            None,
            "returned an interface, and [managed.broken] declares none",
        ),
        (
            "return { interface = 5 }",
            Some("game.broken"),
            "interface is a number, not a table",
        ),
        (
            "return { on = { host = { interfaceOverride = function() end } } }",
            None,
            "on.host.interfaceOverride needs an interface to override",
        ),
    ] {
        let message = refused(source, interface);
        assert!(message.contains(&format!("{at}{problem}")), "{source}:\n{message}");
    }
    let thrown = refused("error('the module itself fails')", None);
    assert!(
        thrown.contains("managed 'broken' of org.example.single (org.example.single@1.0.0/src/broken.jsl) failed to load: ")
            && thrown.contains("the module itself fails"),
        "{thrown}"
    );
}

#[test]
fn a_failing_override_leaves_nothing_published_and_names_its_provider() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("throws");
    single(
        &root,
        &[(
            "breaks",
            "return { on = { host = { interfaceOverride = function() error('no thanks') end } }, interface = {} }\n",
            Some("game.inventory"),
        )],
        "return nil\n",
        &[],
    );
    write(
        &root,
        &[(
            "jess.toml",
            &manifest(
                "org.example.single",
                "[dependencies]\n\"org.example.inventory\" = \"^1.0\"\n\n[managed.breaks]\nmodule = \"src/breaks.jsl\"\ninterface = \"game.inventory\"\n\n[targets.app]\nkind = \"application\"\nentry = \"src/main.jsl\"\n",
            ),
        )],
    );
    let message = launch_error(&request(&root, "app", &[]));
    // Reported once, by the load that failed, not again by the call that raised.
    assert_eq!(message.lines().count(), 2, "{message}");
    assert!(
        message.contains(
            "managed 'breaks' of org.example.single (org.example.single@1.0.0/src/breaks.jsl): on.host.interfaceOverride of 'game.inventory' failed: "
        ) && message.contains("no thanks"),
        "{message}"
    );
}

#[test]
fn failing_host_callbacks_are_attributed_and_the_application_runs_on() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("failing");
    single(
        &root,
        &[(
            "fragile",
            "local updates = 0\nreturn { on = { host = {\n  startup = function() error('startup broke') end,\n  update = function() updates += 1; error(`update {updates} broke`) end,\n} } }\n",
            None,
        )],
        "return nil\n",
        &[],
    );
    let mut application = launch(&request(&root, "app", &[]));
    let started = application.start().unwrap();
    assert_eq!(started.failures.len(), 1);
    let report = application.frame(Duration::from_millis(16)).unwrap();
    assert_eq!(report.failures().len(), 1);
    let lines = application.diagnostics();
    assert!(
        lines[0].starts_with(
            "error in org.example.single/fragile during startup (org.example.single@1.0.0/src/fragile.jsl:3): "
        ) && lines[0].contains("startup broke"),
        "{lines:?}"
    );
    assert!(
        lines[1]
            .starts_with("error in org.example.single/fragile during update (org.example.single@1.0.0/src/fragile.jsl:4): ")
            && lines[1].contains("update 1 broke"),
        "{lines:?}"
    );
    application.frame(Duration::from_millis(16)).unwrap();
    assert!(application.diagnostics()[0].contains("update 2 broke"), "the domain runs on");
    application.stop().unwrap();

    // A load handler or the entry raising fails the launch, attributed: every load handler that
    // raised is reported, by its module's context and where it was defined.
    let entry = temp.path().join("entry");
    single(&entry, &[], "error('the entry broke')\n", &[]);
    let message = launch_error(&request(&entry, "app", &[]));
    assert!(
        message.starts_with("org.example.single target app failed to load:\nerror in org.example.single:app during load: ")
            && message.contains("the entry broke"),
        "{message}"
    );
    let loading = temp.path().join("loading");
    single(
        &loading,
        &[
            (
                "first",
                "return { on = { host = { load = function() error('first broke') end } } }\n",
                None,
            ),
            (
                "second",
                "return { on = { host = { load = function() error('second broke') end } } }\n",
                None,
            ),
        ],
        "return nil\n",
        &[],
    );
    let message = launch_error(&request(&loading, "app", &[]));
    let lines = message.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3, "{message}");
    assert!(
        lines[1].starts_with("error in org.example.single/first during load (org.example.single@1.0.0/src/first.jsl:1): ")
            && lines[1].contains("first broke"),
        "{message}"
    );
    assert!(
        lines[2].starts_with("error in org.example.single/second during load (org.example.single@1.0.0/src/second.jsl:1): ")
            && lines[2].contains("second broke"),
        "{message}"
    );
}

#[test]
fn an_application_holds_its_target_s_capabilities_and_nothing_more() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("authority");
    single(
        &root,
        &[(
            "probe",
            r"local journal = require('@example/journal')
return {
  on = {
    host = {
      startup = function()
        journal.write(`environment {tostring(jess.granted('process.environment'))}`)
        local spawned = require('@jess/worker').spawn {
          capabilities = {},
          source = [[
            return function()
              return require('@jess/interfaces').get('game.inventory')
            end
          ]],
        }
        spawned:send(true)
        -- The worker's failure comes back here, and fails this handler with it.
        spawned:receive(20)
      end,

      update = function() require('../../outside') end,
    },
  },
}
",
            None,
        )],
        "return nil\n",
        &["worker.spawn"],
    );
    // A capability the target declares and the grant withholds stops the launch first.
    let denied = launch_error(&request(&root, "app", &[]));
    assert!(denied.contains("required capability denied:\n    worker.spawn"), "{denied}");
    let mut granted = request(&root, "app", &[]);
    granted.grant = Some(vec!["worker.spawn".to_owned()]);
    let mut application = launch(&granted);

    // It holds worker.spawn and nothing else, and a worker it spawns has no interfaces.
    let started = application.start().unwrap();
    assert_eq!(journal(&application), ["environment false"]);
    assert_eq!(started.failures.len(), 1);
    let worker = application.diagnostics().join("\n");
    assert!(
        worker.contains("interfaces.get: interfaces are the domain's own code's; a worker has none"),
        "{worker}"
    );

    // A managed module requires inside its own package, never past its root.
    let report = application.frame(Duration::from_millis(16)).unwrap();
    assert_eq!(report.failures().len(), 1);
    let escaped = application.diagnostics().join("\n");
    assert!(
        escaped.contains("error in org.example.single/probe during update")
            && escaped.contains("it leaves the package root"),
        "{escaped}"
    );
    application.stop().unwrap();

    // An entrypoint needing more than the target declares refuses activation.
    let layered = temp.path().join("layered");
    write(
        &layered,
        &[
            (
                "jess.toml",
                &manifest(
                    "org.example.layered",
                    "[modules]\n\"@layered/register\" = \"src/register.jsl\"\n\n[[layer]]\nid = \"example.layered\"\n\n[[layer.entrypoint]]\nmodule = \"@layered/register\"\ncapabilities = [\"filesystem.read\"]\n\n[targets.app]\nkind = \"application\"\nentry = \"src/main.jsl\"\n",
                ),
            ),
            ("src/register.jsl", "return nil\n"),
            ("src/main.jsl", "return nil\n"),
        ],
    );
    let refused = launch_error(&request(&layered, "app", &[]));
    assert!(
        refused.contains("required capability denied:\n    filesystem.read"),
        "{refused}"
    );
}

#[test]
fn interfaces_are_looked_up_after_composition_and_only_in_an_application_domain() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("early");
    single(
        &root,
        &[(
            "eager",
            "require('@jess/interfaces').get('game.inventory')\nreturn {}\n",
            None,
        )],
        "return nil\n",
        &[],
    );
    let early = launch_error(&request(&root, "app", &[]));
    assert!(
        early.contains("interfaces.get: interfaces are composed once every managed module has loaded"),
        "{early}"
    );

    // A layer's entrypoint runs before every managed module, so it is early too.
    let layered = temp.path().join("entrypoint");
    write(
        &layered,
        &[
            (
                "jess.toml",
                &manifest(
                    "org.example.layered",
                    "[modules]\n\"@layered/boot\" = \"src/boot.jsl\"\n\n[[layer]]\nid = \"example.layered\"\n\n[[layer.entrypoint]]\nmodule = \"@layered/boot\"\n\n[targets.app]\nkind = \"application\"\nentry = \"src/main.jsl\"\n",
                ),
            ),
            (
                "src/boot.jsl",
                "require('@jess/interfaces').get('game.inventory')\nreturn nil\n",
            ),
            ("src/main.jsl", "return nil\n"),
        ],
    );
    let entrypoint = launch_error(&request(&layered, "app", &[]));
    assert!(
        entrypoint.contains("interfaces.get: interfaces are composed once every managed module has loaded"),
        "{entrypoint}"
    );

    let application = launch(&request(&fixtures().join("game"), "game", &[]));
    let missing = application
        .domain()
        .with_runtime(|runtime| runtime.eval::<()>("require('@jess/interfaces').get('game.missing')"))
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(
        missing.contains("interfaces.get: no package in this domain implements 'game.missing'")
            && missing.contains("it has game.inventory"),
        "{missing}"
    );
    assert_eq!(
        eval(
            &application,
            "return tostring(require('@jess/interfaces').tryGet('game.missing'))"
        ),
        "nil"
    );
    application.stop().unwrap();

    let outside = jess::lua::runtime(Options::default()).unwrap();
    let detached = outside
        .eval::<()>("require('@jess/interfaces').get('game.inventory')")
        .unwrap_err()
        .to_string();
    assert!(detached.contains("interfaces.get: needs a ScriptDomain"), "{detached}");
}

#[test]
fn a_command_target_still_runs_as_a_script_and_run_target_refuses_an_application() {
    let refused = package::run_target(&Options::default(), &request(&fixtures().join("game"), "game", &[])).unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("target 'game' of org.example.game is an application target, which runs in a host of its own"),
        "{refused}"
    );
}

/// A managed module is one package module: a relative `require` of its file, from a layer's
/// entrypoint before the loader runs it or from the entry after, gets the same instance, and
/// its top level runs once.
#[test]
fn a_managed_module_and_a_require_of_its_file_are_one_instance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("shared");
    write(
        &root,
        &[
            (
                "jess.toml",
                &manifest(
                    "org.example.shared",
                    "[dependencies]\n\"org.example.journal\" = \"^1.0\"\n\n[modules]\n\"@shared/boot\" = \"src/boot.jsl\"\n\n[[layer]]\nid = \"example.shared\"\n\n[[layer.entrypoint]]\nmodule = \"@shared/boot\"\n\n[managed.counter]\nmodule = \"src/counter.jsl\"\ninterface = \"game.counter\"\n\n[targets.app]\nkind = \"application\"\nentry = \"src/main.jsl\"\n",
                ),
            ),
            (
                "src/counter.jsl",
                r"local journal = require('@example/journal')
journal.write('counter loaded')
local count = 0
local counter = { bump = function() count += 1; return count end }
return { interface = counter }
",
            ),
            (
                "src/boot.jsl",
                "require('@example/journal').write(`boot bumps to {require('./counter').interface.bump()}`)\nreturn nil\n",
            ),
            (
                "src/main.jsl",
                r"local journal = require('@example/journal')
local required = require('./counter').interface
local composed = require('@jess/interfaces').get('game.counter')
journal.write(`entry: same table {tostring(rawequal(required, composed))}, bumps to {composed.bump()}`)
",
            ),
        ],
    );
    let application = launch(&request(&root, "app", &[]));
    assert_eq!(
        journal(&application),
        ["counter loaded", "boot bumps to 1", "entry: same table true, bumps to 2"]
    );
    application.stop().unwrap();
}

/// Copies the example's packages into `to`, to edit between generations.
fn copy_example(to: &Path) {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    for name in ["journal", "inventory", "armor", "game"] {
        copy(&fixtures().join(name), &to.join(name));
    }
}

/// A restart loads the packages as they are now: contexts run in the new load order, and a
/// module that is gone leaves no context behind.
#[test]
fn a_restart_declares_contexts_in_the_new_load_order() {
    let temp = tempfile::tempdir().unwrap();
    copy_example(temp.path());
    let game = temp.path().join("game");
    let request = TargetRequest {
        package: text(&game),
        target: Some("game".to_owned()),
        paths: vec![text(temp.path())],
        grant: Some(Vec::new()),
        ..TargetRequest::default()
    };
    let mut application = launch(&request);
    application.start().unwrap();

    // The inventory, which everything depends on, gains a module; the game drops its scoreboard.
    let inventory = temp.path().join("inventory");
    let manifest = std::fs::read_to_string(inventory.join("jess.toml")).unwrap();
    std::fs::write(
        inventory.join("jess.toml"),
        format!("{manifest}\n[managed.early]\nmodule = \"src/early.jsl\"\n"),
    )
    .unwrap();
    std::fs::write(
        inventory.join("src/early.jsl"),
        "local journal = require('@example/journal')\nreturn { on = { host = { update = function() journal.write('early update') end } } }\n",
    )
    .unwrap();
    let manifest = std::fs::read_to_string(game.join("jess.toml")).unwrap();
    std::fs::write(
        game.join("jess.toml"),
        manifest.replace("[managed.scoreboard]\nmodule = \"src/scoreboard.jsl\"\n", ""),
    )
    .unwrap();

    application.restart().unwrap();
    let names = application
        .domain()
        .contexts()
        .into_iter()
        .map(|context| context.name)
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "org.example.inventory/early",
            "org.example.inventory/inventory",
            "org.example.armor/armor",
            "org.example.game/director",
        ]
    );
    frame(&mut application);
    let lines = journal(&application);
    assert_eq!(lines[lines.len() - 2..], ["early update", "director update 1 (number)"]);
    application.stop().unwrap();
}

/// However a run ends, it stops the application and passes on what teardown reports: here the
/// run itself fails (the application was already started, so it can't start again), and the
/// failing shutdown handler is still heard from.
#[test]
fn a_run_that_fails_still_stops_and_reports_its_teardown() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("teardown");
    single(
        &root,
        &[(
            "closing",
            "return { on = { host = { shutdown = function() error('shutdown broke') end } } }\n",
            None,
        )],
        "return nil\n",
        &[],
    );
    let application = launch(&request(&root, "app", &[]));
    application.start().unwrap();
    let mut lines = Vec::new();
    let failed = application
        .run(Some(1), &std::sync::atomic::AtomicBool::new(false), |line| {
            lines.push(line.to_owned());
        })
        .unwrap_err()
        .to_string();
    assert!(failed.contains("cannot start"), "{failed}");
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("error in org.example.single/closing during shutdown")
                && line.contains("shutdown broke")),
        "{lines:?}"
    );
}

/// An application launched with an input source gets its action events in its managed modules'
/// `on.host.input`; launched without one, as the command line launches it, it is headless.
#[test]
fn an_application_with_input_runs_its_input_handlers() {
    use jess::host::InputSource;
    use jess::input::{Control, GraphSpec, Key, NodeSpec, Owner, ValueType};

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("input");
    single(
        &root,
        &[(
            "controls",
            r"local journal = require('@example/journal')
return {
  on = {
    host = {
      input = function(payload)
        for _, event in payload.events do
          journal.write(`input {event.action} {event.kind}`)
        end
      end,
    },
  },
}
",
            None,
        )],
        "return nil\n",
        &[],
    );
    let launched = package::launch_with(
        &Options::default(),
        &request(&root, "app", &[]),
        package::LaunchOptions {
            input: Some(InputSource::Synthetic),
        },
    )
    .unwrap();
    let Launched::Application(mut application) = launched else {
        panic!("an application target launches an application");
    };
    application.start().unwrap();
    let keyboard = {
        let mut input = application.host().input().unwrap().lock().unwrap();
        let mut graph = GraphSpec::new();
        graph.action("ui.accept", ValueType::Digital).set("ui", "@test/application");
        graph
            .add_binding(
                &Owner::Set("ui".to_owned()),
                "ui.accept",
                NodeSpec::button(Control::Key(Key::Enter)),
            )
            .unwrap();
        input.install(graph).unwrap();
        input.set_active("ui", true).unwrap();
        input.synthetic().connect_keyboard()
    };
    frame(&mut application);
    {
        let mut input = application.host().input().unwrap().lock().unwrap();
        input.advance(Duration::from_millis(16));
        input.synthetic().press(keyboard, Control::Key(Key::Enter));
    }
    let report = frame(&mut application);
    assert_eq!(report.input.map(|phase| phase.ran), Some(1));
    assert_eq!(journal(&application), ["input ui.accept pressed"]);
    application.stop().unwrap();

    let headless = launch(&request(&root, "app", &[]));
    assert!(headless.host().input().is_none());
    headless.stop().unwrap();
}
