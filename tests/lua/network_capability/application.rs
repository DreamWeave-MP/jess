//! A long-running headless application on the network, run by the jess binary: an application
//! target whose managed module steps one @jess/net loop from `update`, holding a WebSocket open
//! with a heartbeat while a slow HTTPS-shaped request (a JSON POST that takes its time) is in
//! flight, with only the capabilities its target declares granted, and closing everything when it
//! stops: because it asked to, or because it was sent SIGTERM.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use super::http::{Answer, Server, response};
use super::websocket::WsServer;

/// The guide's own example, as the guide shows it: the managed module of a long-running networked
/// application, in "On the network" of the Applications page.
fn monitor_module() -> &'static str {
    const GUIDE: &str = include_str!("../../../site/content/docs/scripting/applications.md");
    let section = &GUIDE[GUIDE.find("## On the network").expect("the guide's section")..];
    let code = &section[section.find("```luau\n").unwrap() + "```luau\n".len()..];
    &code[..code.find("```").unwrap()]
}

const MANIFEST: &str = r#"manifest_version = 1

[package]
id = "org.example.monitor"
version = "1.0.0"
jess_api = "1"
default_target = "monitor"
include = ["src"]

[managed.monitor]
module = "src/monitor.jsl"

[targets.monitor]
kind = "application"
entry = "src/entry.jsl"
capabilities = ["network.dns.resolve", "network.tcp.connect"]
"#;

/// A scratch package for one test, removed when dropped.
struct Monitor(PathBuf);

impl Monitor {
    fn new(name: &str, notifications: &str, request: &str, exit_after_request: bool) -> Self {
        let root = std::env::temp_dir().join(format!("jess-network-{name}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&root));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("jess.toml"), MANIFEST).unwrap();
        std::fs::write(root.join("src/monitor.jsl"), monitor_module()).unwrap();
        std::fs::write(root.join("src/entry.jsl"), "return nil\n").unwrap();
        std::fs::write(
            root.join("src/config.jsl"),
            format!(
                "--!strict\nreturn {{ notifications = {notifications:?}, request = {request:?}, key = 'not-a-real-key', intervalMs = 100, exitAfterRequest = {exit_after_request} }}\n"
            ),
        )
        .unwrap();
        Self(root)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_jess"));
        command.args(["package", "run"]).arg(&self.0);
        command
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

fn granted(command: &mut Command) -> &mut Command {
    command.args(["--capability", "network.dns.resolve", "--capability", "network.tcp.connect"])
}

/// The longest silence between consecutive heartbeats the server received.
fn longest_gap(arrivals: &[(Instant, String)]) -> Duration {
    let beats: Vec<Instant> = arrivals
        .iter()
        .filter(|(_, text)| text.starts_with("heartbeat"))
        .map(|(at, _)| *at)
        .collect();
    beats.windows(2).map(|pair| pair[1] - pair[0]).max().unwrap_or_default()
}

fn output_within(mut child: std::process::Child, limit: Duration) -> Output {
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > limit {
            drop(child.kill());
            panic!("the application was still running after {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn the_networked_application_type_checks_as_a_package() {
    assert!(include_str!("../../../site/content/docs/scripting/applications.md").contains(MANIFEST));
    let monitor = Monitor::new("check", "ws://127.0.0.1:1/", "http://127.0.0.1:1/", false);
    let output = Command::new(env!("CARGO_BIN_EXE_jess"))
        .args(["package", "check"])
        .arg(&monitor.0)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn a_heartbeat_keeps_time_while_a_slow_request_is_in_flight() {
    let notifications = WsServer::start();
    let request = Server::start(vec![(
        "/v1/messages",
        Answer::Slow(Duration::from_millis(1500), response(200, r#"{"content":"hi"}"#, &[])),
    )]);
    let monitor = Monitor::new("heartbeat", &notifications.url("/sink"), &request.url("/v1/messages"), true);

    let child = granted(&mut monitor.command())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = output_within(child, Duration::from_secs(30));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "connected");
    let beats: u32 = lines[1]
        .strip_prefix("request 200 after ")
        .and_then(|rest| rest.strip_suffix(" heartbeats"))
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| panic!("{stdout}"));
    // 15 at 100 ms apart; a busy runner's timers drift, and the gap below is what proves no stall.
    assert!(beats >= 6, "only {beats} heartbeats in a 1.5 s request");
    assert_eq!(lines[2], "shut down");

    assert_eq!(notifications.wait_until_idle(), 0, "the socket was closed");
    let seen = notifications.seen.lock().unwrap();
    let gap = longest_gap(&seen.arrivals);
    assert!(gap < Duration::from_millis(400), "heartbeats stalled for {gap:?}");
    assert_eq!(
        seen.closes,
        [(1001, "shutting down".to_owned())],
        "shutdown closed the socket cleanly"
    );
}

#[test]
fn the_target_s_capabilities_are_granted_explicitly_or_it_does_not_start() {
    let notifications = WsServer::start();
    let monitor = Monitor::new("ungranted", &notifications.url("/sink"), "http://127.0.0.1:1/", true);
    let output = monitor.command().output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr.contains("network.dns.resolve") || stderr.contains("network.tcp.connect"),
        "{stderr}"
    );
    assert_eq!(notifications.seen.lock().unwrap().accepted, 0, "nothing ran");
}

/// SIGTERM stops the application the way its own exit does: shutdown cancels the request that is
/// still in flight, closes the socket with a close frame, and the status is 130.
#[cfg(unix)]
#[test]
fn sigterm_cancels_what_is_in_flight_and_closes_the_socket() {
    use std::io::{BufRead, BufReader, Read};

    let notifications = WsServer::start();
    let request = Server::start(vec![("/v1/messages", Answer::Hang)]);
    let monitor = Monitor::new("sigterm", &notifications.url("/sink"), &request.url("/v1/messages"), false);
    let mut child = granted(&mut monitor.command()).stdout(Stdio::piped()).spawn().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "connected\n");

    let started = Instant::now();
    while notifications.seen.lock().unwrap().arrivals.len() < 3 {
        assert!(started.elapsed() < Duration::from_secs(10), "no heartbeats arrived");
        std::thread::sleep(Duration::from_millis(20));
    }

    let pid = libc::pid_t::try_from(child.id()).unwrap();
    // SAFETY: kill only sends a signal to the child this test spawned and hasn't reaped.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
    let stopped = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if stopped.elapsed() > Duration::from_secs(10) {
            drop(child.kill());
            panic!("SIGTERM did not stop the application");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "shut down\n");
    assert_eq!(status.code(), Some(130));
    assert!(
        stopped.elapsed() < Duration::from_secs(4),
        "shutdown waited on the hung request"
    );
    assert_eq!(notifications.wait_until_idle(), 0);
    assert_eq!(
        notifications.seen.lock().unwrap().closes,
        [(1001, "shutting down".to_owned())]
    );
}
