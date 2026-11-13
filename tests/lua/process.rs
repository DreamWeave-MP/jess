//! `@dream/process` as a jess runtime gives it to a script: bounded runs, started children read
//! and written without blocking, stopping them, who may start them, what is left when the runtime
//! or the domain goes, and that no frame waits for a child.
//!
//! None of them measures time. A limit that is the point of a test is short and is proved by what
//! it reports; every other limit is one only a failing run reaches. l3i's own clocks are no
//! test's either (2.11.0): once a child has exited a run asks the pipe whether anything still
//! holds its capture, and times only a holder it has seen, so a stalled machine never cuts a
//! captured run short.
//!
//! The children are `sh`, `sleep`, `sort` and `cat`, so these are Unix tests.

#![cfg(unix)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use jess::host::{Host, HostSpec};
use jess::lua::Options;
use jess::lua::l3i::Runtime;
use jess::lua::l3i::value::Function;
use jess::runtime::{CapabilitySet, CollectedLogs, ContextSpec, DomainSpec, Phase, RootAuthority, RuntimeFactory};

use super::common;

const SPAWN: &str = "process.spawn";

/// A runtime that may start children and nothing else beyond a runtime's defaults.
fn spawning() -> Runtime {
    jess::lua::runtime_with_capabilities(Options::default(), &[SPAWN.to_owned()]).unwrap_or_else(|error| panic!("{error}"))
}

/// Whether `pid` is a process this one may signal: a child that was killed and reaped is not.
fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Waits up to two seconds for `pid` to be gone.
fn gone(pid: u32) -> bool {
    let started = Instant::now();
    while alive(pid) {
        if started.elapsed() > Duration::from_secs(2) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

/// Starts a child that prints its pid and then sleeps, and answers the pid, leaving the handle in
/// the script's `kept` so that nothing collects it.
const KEEP_A_SLEEPER: &str = r"
    local process = require('@dream/process')
    local child = assert(process.start('sh', { '-c', 'echo $$; exec sleep 30' }, { stdout = 'pipe' }))
    local text = ''
    while not string.find(text, '\n', 1, true) do
      local chunk, message, kind = child:readStdout(64)
      if chunk then
        text ..= chunk
      elseif kind == 'wouldBlock' then
        process.sleep(0.005)
      else
        error(message or 'the child closed its output before printing its pid')
      end
    end
    return child, tonumber((string.gsub(text, '%s', '')))
";

#[test]
fn a_bounded_run_answers_its_result_or_the_limit_that_stopped_it() {
    common::run(
        &spawning(),
        r"
        local process = require('@dream/process')

        local result = assert(process.run('sh', { '-c', 'printf out; printf err >&2; exit 3' },
          { stdout = 'capture', stderr = 'capture', timeoutMs = 60000, maxStdoutBytes = 16, maxStderrBytes = 16 }))
        assert(not result.success and result.code == 3, tostring(result.code))
        assert(result.stdout == 'out' and result.stderr == 'err')

        -- The kind says which ended the run: a child that ran its course answers a result.
        local late, lateMessage, lateKind = process.run('sleep', { '600' }, { timeoutMs = 100 })
        assert(late == nil and lateKind == 'timeout', tostring(lateKind))
        assert(type(lateMessage) == 'string')

        local loud, _, loudKind = process.run('sh', { '-c', 'while :; do echo xxxxxxxxxxxxxxxx; done' },
          { stdout = 'capture', timeoutMs = 60000, maxStdoutBytes = 1024 })
        assert(loud == nil and loudKind == 'outputLimit', tostring(loudKind))

        -- A limit that is no limit is refused before anything starts.
        local ok, problem = pcall(process.run, 'true', {}, { timeoutMs = 0 })
        assert(not ok and string.find(tostring(problem), 'timeoutMs', 1, true), tostring(problem))
        local ceiling = pcall(process.run, 'true', {}, { maxStdoutBytes = 8 })
        assert(not ceiling, 'a ceiling needs a captured stream')
    ",
    );
}

/// l3i 2.10.0: a run answers by its child once a capture is only held by something the child
/// started, says so, and gives a capture that names no ceiling one all the same.
#[cfg(unix)]
#[test]
fn a_run_does_not_wait_on_a_capture_something_else_holds() {
    common::run(
        &spawning(),
        r"
        local process = require('@dream/process')

        -- Had the run waited for the capture to close, nothing would still hold it: `held` is
        -- the child having ended the run, not what it left behind.
        local result = assert(process.run('sh', { '-c', 'sleep 20 & echo out; exit 4' },
          { stdout = 'capture', timeoutMs = 120000 }))
        assert(result.code == 4 and result.stdout == 'out\n', tostring(result.code))
        assert(result.held == true, 'a capture cut short is said to be')

        local whole = assert(process.run('sh', { '-c', 'echo all' }, { stdout = 'capture' }))
        assert(whole.held == nil and whole.stdout == 'all\n')

        local big, _, kind = process.run('head', { '-c', tostring(64 * 1024 * 1024 + 1), '/dev/zero' }, { stdout = 'capture' })
        assert(big == nil and kind == 'outputLimit', tostring(kind))

        local nowhere, message, missing = process.run('true', {}, { cwd = '/no/such/jess/directory' })
        assert(nowhere == nil and missing == 'notFound' and string.find(message, 'cwd', 1, true), tostring(message))
    ",
    );
}

#[test]
fn a_started_child_is_written_to_and_read_from_without_blocking() {
    common::run(
        &spawning(),
        r"
        local process = require('@dream/process')
        local child = assert(process.start('sort', nil, { stdin = 'pipe', stdout = 'pipe', timeoutMs = 60000 }))

        -- Nothing to read yet, and saying so takes no time.
        local early, _, earlyKind = child:readStdout(16)
        assert(early == nil and earlyKind == 'wouldBlock', tostring(earlyKind))
        local running, _, runningKind = child:wait()
        assert(running == nil and runningKind == 'wouldBlock', tostring(runningKind))

        assert(child:write('pear\napple\nfig\n') == 15)
        child:closeStdin()
        local lines = ''
        while true do
          local chunk, message, kind = child:readStdout(4)
          if chunk then
            assert(#chunk >= 1 and #chunk <= 4)
            lines ..= chunk
          elseif kind == 'wouldBlock' then
            process.sleep(0.005)
          elseif message then
            error(message)
          else
            break
          end
        end
        assert(lines == 'apple\nfig\npear\n', lines)
        assert(assert(child:wait(5000)).success)
        child:close()
        child:close()
        assert(not pcall(child.wait, child), 'a closed handle answers nothing')
    ",
    );
}

#[test]
fn a_started_child_is_stopped_by_kill_by_close_and_by_its_limit() {
    let runtime = spawning();
    common::run(
        &runtime,
        r"
        local process = require('@dream/process')

        local killed = assert(process.start('sleep', { '30' }))
        assert(killed:kill() and killed:kill())
        local status = assert(killed:wait(60000))
        assert(not status.success and status.code == nil and type(status.signal) == 'number')
        killed:close()

        local limited = assert(process.start('sh', { '-c', 'sleep 30' }, { stdout = 'pipe', timeoutMs = 100 }))
        local ended
        for _ = 1, 2400 do
          ended = limited:wait(25)
          if ended then break end
        end
        assert(ended and not ended.success, 'the limit stops the child')
        local chunk, _, kind = limited:readStdout(16)
        assert(chunk == nil and kind == 'timeout', tostring(kind))
        limited:close()
    ",
    );
    let pid: u32 = runtime
        .eval(&format!(
            "local child, pid = (function() {KEEP_A_SLEEPER} end)(); child:close(); return pid"
        ))
        .unwrap();
    assert!(gone(pid), "close kills and reaps a child that still runs");
}

#[test]
fn only_a_runtime_granted_process_spawn_starts_anything() {
    common::run(
        &common::runtime(),
        r"
        local process = require('@dream/process')
        assert(jess.granted('process.spawn') == false)
        for _, name in { 'run', 'start' } do
          local ok, problem = pcall(process[name], 'true', {})
          assert(not ok, `{name} started a child without the capability`)
          assert(string.find(tostring(problem), `'process.spawn' capability`, 1, true), tostring(problem))
        end
    ",
    );
}

/// A domain's runtimes are built from its grant, as `jess package run` builds an application's, so
/// a domain not granted `process.spawn` starts nothing. The check is the runtime plan's: l3i owns
/// this module, and the plan is where its capabilities are decided.
#[test]
fn a_domain_starts_children_only_when_its_grant_and_its_runtime_carry_process_spawn() {
    for (granted, expected) in [(false, "false false"), (true, "true true")] {
        let names: Vec<String> = if granted { vec![SPAWN.to_owned()] } else { Vec::new() };
        let logs = Arc::new(CollectedLogs::new());
        let grant = RootAuthority::new().grant(CapabilitySet::from_names(&names).unwrap());
        let factory: Arc<dyn RuntimeFactory> =
            Arc::new(move || jess::lua::runtime_with_capabilities(Options::default(), &names));
        let domain =
            jess::runtime::ScriptDomain::create(factory, DomainSpec::new("spawning", grant).logs(logs.clone())).unwrap();
        domain
            .register_context(ContextSpec::new("main").schedule([Phase::Startup]))
            .unwrap();
        domain
            .load_source(
                "@test/process/granted.jsl",
                r"
                local process = require('@dream/process')
                local ran = pcall(process.run, 'true', {}, { timeoutMs = 60000 })
                local started, child = pcall(process.start, 'true', {})
                if started and child then child:close() end
                require('@jess/log').new('spawning').info(`{jess.granted('process.spawn')} {ran and started}`)
                ",
            )
            .unwrap();
        let messages = logs.drain().into_iter().map(|record| record.message).collect::<Vec<_>>();
        assert_eq!(messages, [expected]);
    }
}

#[test]
fn no_child_outlives_the_runtime_that_started_it() {
    let runtime = spawning();
    // Globals are read-only, so the handles live in a closure this test holds: nothing collects
    // them, and it is the runtime's shutdown that ends the children.
    let keep: Function = runtime
        .eval(&format!(
            r"local first, firstPid = (function() {KEEP_A_SLEEPER} end)()
              local second, secondPid = (function() {KEEP_A_SLEEPER} end)()
              return function() return firstPid, secondPid, first, second end"
        ))
        .unwrap();
    let (first, second) = keep.invoke::<(u32, u32), ()>(&runtime.stack(), ()).unwrap();
    assert!(alive(first) && alive(second), "the children run while their runtime does");
    drop(keep);
    drop(runtime);
    assert!(
        gone(first) && gone(second),
        "shutting the runtime down kills and reaps what it started"
    );
}

/// Nor the domain: a handle a context still holds when the domain stops goes with its runtime.
#[test]
fn no_child_outlives_the_domain_that_started_it() {
    let logs = Arc::new(CollectedLogs::new());
    let grant = RootAuthority::new().grant(CapabilitySet::from_names([SPAWN]).unwrap());
    let factory: Arc<dyn RuntimeFactory> =
        Arc::new(|| jess::lua::runtime_with_capabilities(Options::default(), &[SPAWN.to_owned()]));
    let mut host = Host::create(factory, HostSpec::new(DomainSpec::new("leaving", grant).logs(logs.clone()))).unwrap();
    host.domain()
        .register_context(ContextSpec::new("main").schedule([Phase::Update]))
        .unwrap();
    host.domain()
        .load_source(
            "@test/process/leaving.jsl",
            &format!(
                r"local child, pid = (function() {KEEP_A_SLEEPER} end)()
                  require('@jess/log').new('leaving').info(tostring(pid))
                  require('@jess/context').bind('main', {{ update = function() return child end }})"
            ),
        )
        .unwrap();
    host.domain().start().unwrap();
    host.frame(Duration::from_millis(16)).unwrap();
    let pid: u32 = logs.drain()[0].message.parse().unwrap();
    assert!(alive(pid), "the child runs while its domain does");
    host.stop().unwrap();
    assert!(gone(pid), "stopping the domain kills and reaps what it started");
}

/// A frame-driven host polls: no frame waits for a child to finish. The child here can't finish
/// until its input closes, and the script closes it only after five frames have each asked
/// whether it finished and been told not yet. A frame that waited for it would wait until the
/// child's own limit killed it, and the test would then fail for want of its output.
///
/// It does not say how long a frame takes: a wait that gives up after a while and answers "not
/// yet" passes here, and nothing short of a clock would catch it.
#[test]
fn no_frame_waits_for_a_running_child() {
    let logs = Arc::new(CollectedLogs::new());
    let factory: Arc<dyn RuntimeFactory> =
        Arc::new(|| jess::lua::runtime_with_capabilities(Options::default(), &[SPAWN.to_owned()]));
    let grant = RootAuthority::new().grant(CapabilitySet::from_names([SPAWN]).unwrap());
    let mut host = Host::create(factory, HostSpec::new(DomainSpec::new("pacing", grant).logs(logs.clone()))).unwrap();
    host.domain()
        .register_context(ContextSpec::new("main").schedule([Phase::Update, Phase::Shutdown]))
        .unwrap();
    host.domain()
        .load_source(
            "@test/process/pacing.jsl",
            r"
            local process = require('@dream/process')
            local log = require('@jess/log').new('pacing')
            local child = assert(process.start(
              'sh',
              { '-c', 'cat >/dev/null; echo done' },
              { stdin = 'pipe', stdout = 'pipe', timeoutMs = 30000 }
            ))
            local output = ''
            local unanswered = 0
            local finished = false

            require('@jess/context').bind('main', {
              update = function()
                if finished then return end

                local chunk = child:readStdout(4096)
                if chunk then output ..= chunk end

                local status, problem, kind = child:wait()
                assert(status or kind == 'wouldBlock', `{kind}: {problem}`)
                if status and string.find(output, 'done', 1, true) then
                  finished = true
                  child:close()
                  log.info(`finished {status.success} after {unanswered} frames without an answer`)
                  return
                end

                if status then return end

                unanswered += 1
                if unanswered == 5 then child:closeStdin() end
              end,
            })
            ",
        )
        .unwrap();
    host.domain().start().unwrap();

    // The deadline is looked at between frames, so it ends a run whose frames return and never
    // finish; a frame that never returns is ended by the child's limit, which is shorter.
    let started = Instant::now();
    let mut messages = Vec::new();
    while messages.is_empty() && started.elapsed() < Duration::from_secs(60) {
        host.frame(Duration::from_millis(2)).unwrap();
        messages.extend(logs.drain().into_iter().map(|record| record.message));
        std::thread::yield_now();
    }
    host.stop().unwrap();

    let [message] = &messages[..] else {
        panic!("the child never finished: {messages:?}");
    };
    let unanswered: u32 = message
        .strip_prefix("finished true after ")
        .and_then(|rest| rest.strip_suffix(" frames without an answer"))
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| panic!("{message}"));
    assert!(
        unanswered >= 5,
        "the child finished after {unanswered} frames, before its input closed"
    );
}

/// The declarations jess hands a checker come from the l3i it was built with: the bounded run and
/// the handle type-check under `--!strict`, and a method the handle lacks does not.
#[test]
fn the_types_describe_bounded_runs_and_process_handles() {
    let good = common::type_check(
        r"--!strict
        local process = require('@dream/process')
        local result, message, kind = process.run('true', {}, { stdout = 'capture', timeoutMs = 100, maxStdoutBytes = 64 })
        if result then
          local _code: number? = result.code
        elseif kind == 'timeout' or kind == 'outputLimit' then
          local _text: string? = message
        end
        local child = process.start('cat', nil, { stdin = 'pipe', stdout = 'pipe', stderr = 'null', timeoutMs = 100 })
        if child then
          local _queued: number? = child:write('x')
          child:closeStdin()
          local _chunk: (buffer | string)? = child:readStdout(16)
          local status = child:wait(10)
          if status then
            local _success: boolean = status.success
          end
          child:kill()
          child:close()
        end
        ",
    );
    assert_eq!(good, "", "{good}");
    let bad = common::type_check(
        r"--!strict
        local child = assert(require('@dream/process').start('true'))
        child:detach()
        ",
    );
    assert!(bad.contains("detach"), "{bad}");
}

/// A secret for a child (l3i 2.9.0): it arrives on a descriptor the child inherits, named by
/// number in a variable, readable once, and is in neither the child's arguments nor its
/// environment. It needs `process.spawn` and nothing more, and under a domain it is the domain's
/// grant that says whether anything is started at all.
#[test]
fn a_secret_reaches_a_child_once_and_only_through_its_descriptor() {
    common::run(
        &spawning(),
        r#"
        local process = require('@dream/process')
        local secret = 'jess-ZZ-SECRET-ZZ'
        local result = assert(process.run('sh', { '-c',
          'printf "%s|" "$JESS_SECRET_FD"; cat "/dev/fd/$JESS_SECRET_FD"; printf "|"; cat "/dev/fd/$JESS_SECRET_FD"; printf "|"; env; echo "$0" "$@"' },
          { secret = { env = 'JESS_SECRET_FD', data = secret }, stdout = 'capture', timeoutMs = 60000 }))
        local number, first, second, rest = string.match(result.stdout, '^(%d+)|(.-)|(.-)|(.*)$')
        assert(tonumber(number) >= 3, result.stdout)
        assert(first == secret, 'the child did not read the secret')
        assert(second == '', 'the secret could be read twice')
        assert(string.find(rest, 'JESS_SECRET_FD=', 1, true) and not string.find(rest, 'ZZ-SECRET-ZZ', 1, true),
          'the secret is in the child\'s environment or arguments')

        -- A started child takes one the same way.
        local child = assert(process.start('sh', { '-c', 'cat "/dev/fd/$JESS_SECRET_FD"' },
          { secret = { env = 'JESS_SECRET_FD', data = buffer.fromstring(secret) }, stdout = 'pipe', timeoutMs = 60000 }))
        local text = ''
        while true do
          local chunk, message, kind = child:readStdout(4096)
          if chunk then
            text ..= chunk
          elseif kind == 'wouldBlock' then
            process.sleep(0.005)
          elseif message then
            error(message)
          else
            break
          end
        end
        child:close()
        assert(text == secret, text)

        -- Malformed ones raise before anything starts, and say nothing of the bytes.
        for _, bad in { { env = 'lower', data = secret }, { env = 'PATH', data = secret }, { env = 'FD', data = '' }, { env = 'FD' } } do
          local ok, problem = pcall(process.run, 'true', {}, { secret = bad })
          assert(not ok and not string.find(tostring(problem), 'ZZ-SECRET-ZZ', 1, true), tostring(problem))
        end
    "#,
    );
    // Without the capability nothing is started, secret or not.
    common::run(
        &common::runtime(),
        r"
        local process = require('@dream/process')
        local ok, problem = pcall(process.run, 'true', {}, { secret = { env = 'FD', data = 'x' } })
        assert(not ok and string.find(tostring(problem), `'process.spawn' capability`, 1, true), tostring(problem))
    ",
    );
    // And the declarations a checker is handed know the option.
    let typed = common::type_check(
        r"--!strict
        local process = require('@dream/process')
        local _run = process.run('true', {}, { secret = { env = 'FD', data = 'x' }, timeoutMs = 100 })
        local _start = process.start('true', nil, { secret = { env = 'FD', data = buffer.fromstring('x') } })
        ",
    );
    assert_eq!(typed, "", "{typed}");
    let mistyped = common::type_check(
        r"--!strict
        local _run = require('@dream/process').run('true', {}, { secret = { env = 'FD' } })
        ",
    );
    assert!(!mistyped.is_empty(), "a secret without data type-checked");
}
