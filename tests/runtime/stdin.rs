//! Stdin under a domain: what a context may open, what a handle hands over, what a frame-driven
//! scheduler refuses, and when the process-wide claim comes free. Stdin is one per process, so each
//! case runs in a child of this test binary, with a pipe for stdin that the parent controls: a
//! test never reads the terminal or the test runner's stdin.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use jess::lua::{Options, runtime_with_capabilities};
use jess::runtime::l3i::{self, Runtime};
use jess::runtime::{CapabilitySet, ContextSpec, Phase, RuntimeFactory};

use super::support;

const CHILD: &str = "JESS_STDIN_TEST_CHILD";

/// Real jess runtimes, the application host's way: a worker's VM holds what it was delegated.
struct Factory;

impl RuntimeFactory for Factory {
    fn create(&self) -> l3i::Result<Runtime> {
        runtime_with_capabilities(Options::default(), &["process.stdio".to_owned(), "worker.spawn".to_owned()])
    }

    fn create_worker(&self, capabilities: &CapabilitySet) -> l3i::Result<Runtime> {
        let names = capabilities
            .iter()
            .map(|capability| capability.name().to_owned())
            .collect::<Vec<_>>();
        runtime_with_capabilities(Options::default(), &names)
    }
}

fn harness(name: &str) -> support::Harness {
    support::domain_from(Arc::new(Factory), name, &["process.stdio", "worker.spawn"])
}

/// Runs the test named `case` again in a child, with `input` on its stdin. The pipe stays open
/// until the child ends when `input` says nothing, as an idle terminal would.
fn in_child(case: &str, input: Option<&[u8]>) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("runtime::stdin::{case}"), "--nocapture"])
        .env(CHILD, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take();
    if let Some(input) = input {
        pipe.as_mut().unwrap().write_all(input).unwrap();
    }
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{case}: {status}");
            break;
        }
        if Instant::now() >= until {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("{case} hung");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(pipe);
}

/// True in the child, which does the work; the parent calls [`in_child`] and returns.
fn is_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

#[test]
fn a_handed_handle_delegates_reading_but_not_opening_or_waiting() {
    if !is_child() {
        return in_child(
            "a_handed_handle_delegates_reading_but_not_opening_or_waiting",
            Some(b"delegated\n"),
        );
    }
    let harness = harness("delegation");
    let domain = &harness.domain;
    domain
        .register_context(ContextSpec::new("opener").schedule([Phase::Startup]))
        .unwrap();
    domain
        .register_context(
            ContextSpec::new("restricted")
                .schedule([Phase::Startup])
                .capabilities(CapabilitySet::empty()),
        )
        .unwrap();
    domain
        .load_source(
            "=delegation",
            r"
        local context = require('@jess/context')
        local process = require('@dream/process')
        local shared
        context.bind('opener', { startup = function()
            shared = assert(process.openStdin())
        end })
        context.bind('restricted', { startup = function()
            local opened, denied = pcall(process.openStdin)
            assert(not opened and denied:find('process.stdio', 1, true), denied)
            local waited, refused = pcall(shared.wait, shared, 0)
            assert(not waited and refused:find('would block', 1, true), refused)
            local line, message, kind
            local started = os.clock()
            repeat
                line, message, kind = shared:readLine()
            until kind ~= 'wouldBlock' or os.clock() - started > 20
            assert(line == 'delegated', message)
            shared:close()
        end })
    ",
        )
        .unwrap();
    let report = domain.start().unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
}

#[test]
fn restart_and_stop_release_a_claim_the_script_never_closed() {
    // A bound handler keeps the handle reachable: nothing but the generation's end can free it.
    const HOLD: &str = "local held = assert(require('@dream/process').openStdin()) \
         require('@jess/context').bind('keeper', { startup = function() return held end })";
    if !is_child() {
        return in_child("restart_and_stop_release_a_claim_the_script_never_closed", None);
    }
    let harness = harness("release");
    let domain = &harness.domain;
    domain
        .register_context(ContextSpec::new("keeper").schedule([Phase::Startup]))
        .unwrap();
    domain.load_source("=first", HOLD).unwrap();
    domain.start().unwrap();
    domain.restart(|runtime| runtime.exec(HOLD)).unwrap();
    domain.stop().unwrap();
    runtime_with_capabilities(Options::default(), &["process.stdio".to_owned()])
        .unwrap()
        .exec("assert(require('@dream/process').openStdin()):close()")
        .unwrap();
}

#[test]
fn a_worker_holds_the_one_claim_only_if_delegated_and_gives_it_back_when_it_ends() {
    if !is_child() {
        return in_child(
            "a_worker_holds_the_one_claim_only_if_delegated_and_gives_it_back_when_it_ends",
            None,
        );
    }
    let harness = harness("worker");
    harness
        .domain
        .load_source(
            "=worker",
            r"
        local process = require('@dream/process')
        local worker = require('@jess/worker')
        local denied = worker.spawn({ capabilities = {}, source = [[
            local opened, message = pcall(require('@dream/process').openStdin)
            return function() return { opened = opened, message = message } end
        ]] })
        denied:send('report')
        local answer = denied:receive(20)
        assert(not answer.opened and answer.message:find('process.stdio', 1, true), answer.message)
        denied:close()
        denied:join(20)

        local holder = worker.spawn({ capabilities = { 'process.stdio' }, source = [[
            local held = assert(require('@dream/process').openStdin())
            return function()
                local waited, refused = pcall(held.wait, held, 0)
                return { waited = waited, refused = refused }
            end
        ]] })
        holder:send('report')
        local report = holder:receive(20)
        assert(not report.waited and report.refused:find('would block', 1, true), report.refused)

        local second, message, kind = process.openStdin()
        assert(second == nil and kind == 'wouldBlock', message)
        holder:close()
        holder:join(20)
        assert(process.openStdin()):close()
    ",
        )
        .unwrap();
}
