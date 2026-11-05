//! Timers execute as context-owned scheduler work, driven entirely by simulated time.
use super::support;
use jess::runtime::{Budget, CapabilitySet, ContextSpec, DomainSpec, Phase, RootAuthority, ScriptDomain};
use std::time::Duration;

#[test]
fn timer_callbacks_are_ordered_cancellable_and_not_recursive() {
    let harness = support::domain("timers", &[]);
    let domain = &harness.domain;
    domain
        .load_source(
            "=timers",
            r"
        local timer = require('@jess/timer')
        local journal = require('@test/journal')
        local cancelled = timer.after(0, function() error('cancelled fired') end)
        assert(cancelled:cancel() and not cancelled:cancel() and not cancelled.active)
        local second
        timer.after(10, function()
            journal.write('first')
            assert(second:cancel())
            timer.after(0, function() journal.write('nested') end)
        end)
        second = timer.after(10, function() error('cancelled batch item fired') end)
        timer.after(10, function() journal.write('third') end)
        local repeating
        repeating = timer.every(5, function()
            journal.write('repeat')
            repeating:cancel()
        end)
    ",
        )
        .unwrap();
    domain.start().unwrap();
    domain.advance(Duration::from_millis(10)).unwrap();
    assert_eq!(support::journal(), ["repeat", "first", "third"]);
    domain.advance(Duration::ZERO).unwrap();
    assert_eq!(support::journal(), ["nested"]);
    domain.advance(Duration::from_secs(100)).unwrap();
    assert_eq!(support::journal().len(), 0);
}

#[test]
fn timer_errors_budgets_and_authority_are_the_owners() {
    let harness = support::domain("authority", &[support::TEST_ECHO]);
    let domain = &harness.domain;
    let id = domain
        .register_context(
            ContextSpec::new("restricted")
                .schedule([Phase::Startup, Phase::Timer])
                .capabilities(CapabilitySet::empty())
                .budget(Budget::time(Duration::from_millis(20))),
        )
        .unwrap();
    domain
        .load_source(
            "=authority",
            r"
        local context = require('@jess/context')
        local timer = require('@jess/timer')
        local echo = require('@test/echo')
        context.bind('restricted', { startup = function()
            timer.after(0, function() echo.echo(1) end)
            timer.after(0, function() while true do end end)
            timer.after(0, function() require('@test/journal').write('survived') end)
        end })
    ",
        )
        .unwrap();
    domain.start().unwrap();
    let report = domain.advance(Duration::ZERO).unwrap();
    let timers = report.phases.iter().find(|phase| phase.phase == Phase::Timer).unwrap();
    assert_eq!(timers.ran, 3);
    assert_eq!(timers.failures.len(), 2);
    assert_eq!(domain.context("restricted").unwrap().id, id);
    assert_eq!(domain.context("restricted").unwrap().failures, 2);
    assert_eq!(support::journal(), ["survived"]);
}

#[test]
fn restart_removes_old_timers_and_resets_the_clock() {
    let harness = support::domain("restart", &[]);
    let domain = &harness.domain;
    domain
        .load_source(
            "=before",
            "require('@jess/timer').every(5, function() error('stale timer') end)",
        )
        .unwrap();
    domain.start().unwrap();
    domain.advance(Duration::from_millis(4)).unwrap();
    domain.restart(|runtime| runtime.exec("local timer = require('@jess/timer') assert(timer.now() == 0) timer.after(0, function() require('@test/journal').write('fresh') end)")).unwrap();
    let report = domain.advance(Duration::from_secs(10)).unwrap();
    assert!(report.phases.iter().all(|phase| phase.failures.is_empty()));
    assert_eq!(support::journal(), ["fresh"]);
}

#[test]
fn invalid_delays_and_unscheduled_contexts_are_refused() {
    let harness = support::domain("invalid", &[]);
    let domain = &harness.domain;
    domain
        .register_context(ContextSpec::new("untimed").schedule([Phase::Startup]))
        .unwrap();
    domain
        .load_source(
            "=invalid",
            r"
        local timer = require('@jess/timer')
        for _, value in {-1, 0/0, math.huge, 31536000001} do
            assert(not pcall(timer.after, value, function() end))
        end
        assert(not pcall(timer.every, 0, function() end))
        require('@jess/context').bind('untimed', { startup = function()
            local ok, message = pcall(timer.after, 1, function() end)
            assert(not ok and message:find('does not schedule the timer phase', 1, true), message)
        end })
    ",
        )
        .unwrap();
    assert_eq!(domain.start().unwrap().failures.len(), 0);
}

#[test]
fn repeats_coalesce_long_frames_and_idle_domains_add_no_timer_phase() {
    let harness = support::domain("repeat", &[]);
    let domain = &harness.domain;
    domain
        .load_source(
            "=repeat",
            r"
        require('@jess/timer').every(10, function() require('@test/journal').write('tick') end)
    ",
        )
        .unwrap();
    domain.start().unwrap();
    let report = domain.advance(Duration::from_millis(9)).unwrap();
    assert!(!report.phases.iter().any(|phase| phase.phase == Phase::Timer));
    domain.advance(Duration::from_secs(100)).unwrap();
    assert_eq!(support::journal(), ["tick"]);
    domain.advance(Duration::ZERO).unwrap();
    assert_eq!(support::journal().len(), 0);
    domain.advance(Duration::from_millis(10)).unwrap();
    assert_eq!(support::journal(), ["tick"]);
}

#[test]
fn thousands_of_due_timers_arrive_in_order_across_bounded_frames() {
    let harness = support::domain("stress", &[]);
    let domain = &harness.domain;
    domain
        .load_source(
            "=stress",
            r"
        local timer = require('@jess/timer')
        local journal = require('@test/journal')
        local seen = 0
        for index = 1, 5000 do
            timer.after(index % 7, function()
                seen += 1
                if seen % 1000 == 0 then journal.write(tostring(seen)) end
            end)
        end
        -- Churn leaves nothing behind: every one of these is gone before the limit could notice.
        for _ = 1, 40000 do
            assert(timer.after(1000, function() error('cancelled fired') end):cancel())
        end
    ",
        )
        .unwrap();
    domain.start().unwrap();
    let mut ran = Vec::new();
    for _ in 0..5 {
        let report = domain.advance(Duration::from_millis(7)).unwrap();
        assert_eq!(report.failures().len(), 0);
        ran.push(report.phases.iter().find(|phase| phase.phase == Phase::Timer).unwrap().ran);
    }
    assert_eq!(ran, [1024, 1024, 1024, 1024, 904]);
    assert_eq!(support::journal(), ["1000", "2000", "3000", "4000", "5000"]);
}

#[test]
fn the_domain_limit_refuses_with_a_message_and_recovers() {
    let harness = support::domain("limit", &[]);
    harness
        .domain
        .load_source(
            "=limit",
            r"
        local timer = require('@jess/timer')
        local handles = {}
        for index = 1, 16384 do
            handles[index] = timer.after(1000, function() end)
        end
        local ok, message = pcall(timer.after, 1000, function() end)
        assert(not ok and message:find('active timer limit', 1, true), message)
        assert(handles[1]:cancel())
        timer.after(1000, function() end)
    ",
        )
        .unwrap();
}

#[test]
fn a_failing_repeat_keeps_repeating_and_its_failure_names_where_it_was_scheduled() {
    let harness = support::domain("failing", &[]);
    let domain = &harness.domain;
    domain
        .load_source(
            "=failing",
            r"
        require('@jess/timer').every(5, function()
            require('@test/journal').write('tick')
            error('tick failed')
        end)
    ",
        )
        .unwrap();
    domain.start().unwrap();
    for _ in 0..2 {
        let report = domain.advance(Duration::from_millis(5)).unwrap();
        assert_eq!(report.failures().len(), 1);
    }
    assert_eq!(support::journal(), ["tick", "tick"]);
    let diagnostics = harness.diagnostics.drain();
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    let failure = &diagnostics[0];
    assert_eq!(failure.phase, Some(Phase::Timer));
    assert!(failure.message.contains("tick failed"), "{failure:?}");
    assert!(
        failure.source.as_deref().is_some_and(|source| source.contains("failing:2")),
        "{failure:?}"
    );
}

#[test]
fn a_timer_can_cancel_itself_and_a_one_shot_is_active_until_it_returns() {
    let harness = support::domain("self", &[]);
    let domain = &harness.domain;
    domain
        .load_source(
            "=self",
            r"
        local timer = require('@jess/timer')
        local journal = require('@test/journal')
        local once
        once = timer.after(1, function()
            journal.write(`during {once.active}`)
        end)
        local twice = 0
        local repeating
        repeating = timer.every(1, function()
            twice += 1
            journal.write(`repeat {repeating:cancel()} {repeating:cancel()} {repeating.active}`)
        end)
        require('@test/scratch').once = once
    ",
        )
        .unwrap();
    domain.start().unwrap();
    domain.advance(Duration::from_millis(1)).unwrap();
    assert_eq!(support::journal(), ["during true", "repeat true false false"]);
    domain.advance(Duration::from_secs(1)).unwrap();
    assert_eq!(support::journal().len(), 0);
    assert!(!support::eval::<bool>(domain, "return require('@test/scratch').once.active"));
}

#[test]
fn nothing_outlives_its_generation_or_a_failed_load() {
    let harness = support::domain("failed load", &[]);
    let domain = &harness.domain;
    let failed = domain.load_source(
        "=failed",
        "require('@jess/timer').after(0, function() require('@test/journal').write('stale') end) error('no')",
    );
    assert!(failed.is_err());
    domain.stop().unwrap();
    assert_eq!(support::journal().len(), 0);

    let harness = support::domain("stopped", &[]);
    let domain = &harness.domain;
    domain
        .load_source(
            "=stopped",
            r"
        local context = require('@jess/context')
        local timer = require('@jess/timer')
        handle = timer.after(1000, function() end)
        late = timer.every(1, function() require('@test/journal').write('stale') end)
    ",
        )
        .unwrap();
    domain.start().unwrap();
    domain.stop().unwrap();
    assert_eq!(support::journal().len(), 0);
    assert!(domain.advance(Duration::from_secs(1)).is_err());
}

#[test]
fn a_timer_scheduled_while_stopping_is_refused_not_lost_silently() {
    let harness = support::domain("stopping", &[]);
    let domain = &harness.domain;
    domain
        .register_context(ContextSpec::new("late").schedule([Phase::Shutdown, Phase::Timer]))
        .unwrap();
    domain
        .load_source(
            "=stopping",
            r"
        require('@jess/context').bind('late', { shutdown = function()
            local ok, message = pcall(require('@jess/timer').after, 0, function() end)
            require('@test/journal').write(`{ok} {message:find('schedule a timer', 1, true) ~= nil}`)
        end })
    ",
        )
        .unwrap();
    domain.start().unwrap();
    domain.stop().unwrap();
    assert_eq!(support::journal(), ["false true"]);
}

#[test]
fn a_worker_is_told_it_has_no_frames() {
    let harness = support::domain("worker", &["worker.spawn"]);
    harness
        .domain
        .load_source(
            "=worker",
            r"
        local worker = require('@jess/worker').spawn({ source = [[
            local ok, message = pcall(require('@jess/timer').after, 0, function() end)
            return function() return { ok = ok, message = message } end
        ]] })
        worker:send('report')
        local answer = worker:receive(20)
        assert(not answer.ok and answer.message:find('@jess/timer.after is for a domain', 1, true), answer.message)
        worker:close()
        worker:join(20)
    ",
        )
        .unwrap();
}

/// A budget of zero is the one a test can state without a clock: every callback ends past it.
#[test]
fn a_dispatch_budget_defers_the_rest_of_the_batch_in_order() {
    let grant = RootAuthority::new().grant(CapabilitySet::empty());
    let spec = DomainSpec::new("budgeted", grant).timer_dispatch_budget(Duration::ZERO);
    let domain = ScriptDomain::create(support::factory(), spec).unwrap();
    domain
        .load_source(
            "=budgeted",
            r"
        local timer = require('@jess/timer')
        local journal = require('@test/journal')
        local cancelled
        timer.after(0, function()
            journal.write('first')
            assert(cancelled:cancel())
            timer.after(0, function() journal.write('newborn') end)
        end)
        cancelled = timer.after(0, function() journal.write('cancelled') end)
        timer.after(0, function() journal.write('third') end)
    ",
        )
        .unwrap();
    domain.start().unwrap();
    for expected in ["first", "third", "newborn"] {
        let report = domain.advance(Duration::ZERO).unwrap();
        assert_eq!(support::journal(), [expected]);
        let timers = report.phases.iter().find(|phase| phase.phase == Phase::Timer).unwrap();
        assert_eq!(timers.ran, 1);
    }
    let report = domain.advance(Duration::ZERO).unwrap();
    assert!(!report.phases.iter().any(|phase| phase.phase == Phase::Timer));
}

/// Not a check: what timers cost, for a verified build.
/// `cargo test --release --test integration --all-features -- --ignored --nocapture --exact runtime::timers::measure_timer_overhead`,
/// in a shell where `tools/dev/no-lto.sh` was not sourced.
#[test]
#[ignore = "a measurement, meaningful only on a verified release build"]
fn measure_timer_overhead() {
    const FRAMES: u32 = 200_000;
    let idle = support::domain("idle", &[]);
    idle.domain.load_source("=idle", "").unwrap();
    idle.domain.start().unwrap();
    let started = std::time::Instant::now();
    for _ in 0..FRAMES {
        idle.domain.advance(Duration::ZERO).unwrap();
    }
    println!(
        "idle advance, no timers: {:.1} ns",
        started.elapsed().as_secs_f64() * 1e9 / f64::from(FRAMES)
    );

    for count in [100u32, 16_384] {
        let parked = support::domain("parked", &[]);
        parked
            .domain
            .load_source(
                "=parked",
                &format!("local timer = require('@jess/timer') for _ = 1, {count} do timer.after(1e9, function() end) end"),
            )
            .unwrap();
        parked.domain.start().unwrap();
        let started = std::time::Instant::now();
        for _ in 0..FRAMES {
            parked.domain.advance(Duration::ZERO).unwrap();
        }
        println!(
            "idle advance, {count} timers not due: {:.1} ns",
            started.elapsed().as_secs_f64() * 1e9 / f64::from(FRAMES)
        );

        let churn = support::domain("churn", &[]);
        churn
            .domain
            .load_source(
                "=churn",
                &format!(
                    "local timer = require('@jess/timer')
                 local noop = function() end
                 local handles = table.create({count})
                 local started = os.clock()
                 for index = 1, {count} do handles[index] = timer.after(index % 1000 + 1, noop) end
                 local scheduled = os.clock()
                 for index = 1, {count} do handles[index]:cancel() end
                 require('@test/journal').write(string.format('schedule %.0f ns each, cancel %.0f ns each',
                     (scheduled - started) / {count} * 1e9, (os.clock() - scheduled) / {count} * 1e9))"
                ),
            )
            .unwrap();
        println!("{count} timers: {}", support::journal().join(" "));

        let due = support::domain("due", &[]);
        due.domain
            .load_source(
                "=due",
                &format!(
                    "local timer = require('@jess/timer') local noop = function() end
                     for _ = 1, {count} do timer.every(1, noop) end"
                ),
            )
            .unwrap();
        due.domain.start().unwrap();
        let started = std::time::Instant::now();
        let mut ran = 0usize;
        for _ in 0..500 {
            let report = due.domain.advance(Duration::from_millis(1)).unwrap();
            ran += report
                .phases
                .iter()
                .filter(|phase| phase.phase == Phase::Timer)
                .map(|phase| phase.ran)
                .sum::<usize>();
        }
        println!(
            "{count} repeating timers: {:.0} ns per callback",
            started.elapsed().as_secs_f64() * 1e9 / f64::from(u32::try_from(ran).unwrap())
        );
    }
}
