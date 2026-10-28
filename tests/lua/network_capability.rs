use jess::lua::Options;
use jess::lua::l3i::Runtime;

mod http;
mod websocket;

fn runtime(capabilities: &[String]) -> Runtime {
    jess::lua::runtime_with_capabilities(Options::default(), capabilities).unwrap()
}

/// The test CA, and the certificate it signed for `localhost` and `127.0.0.1` with its key, valid
/// from 2026 to 2126. Test fixtures only: the key is public.
const CA: &str = include_str!("network_capability/fixtures/ca.pem");
const SERVER_CERT: &str = include_str!("network_capability/fixtures/server.pem");
const SERVER_KEY: &str = include_str!("network_capability/fixtures/server.key");
const TLS_FRONT: &str = include_str!("network_capability/fixtures/tls_front.jsl");

/// A runtime that may resolve names and connect out.
fn network_runtime() -> Runtime {
    runtime(&["network.dns.resolve".to_owned(), "network.tcp.connect".to_owned()])
}

/// A runtime that may also listen on loopback, for the TLS front.
fn tls_runtime() -> Runtime {
    runtime(&[
        "network.dns.resolve".to_owned(),
        "network.tcp.connect".to_owned(),
        "network.tcp.listen".to_owned(),
    ])
}

/// Runs `body` as a task on a fresh @jess/net loop until it ends, closes the loop, and returns the
/// lines the body put in `out`. The body sees the loop as `loop`, `@jess/http` as `http`, the TLS
/// front's `tlsFront`, and the certificates as `CA`, `SERVER_CERT` and `SERVER_KEY`.
fn run_task(runtime: &Runtime, body: &str) -> Vec<String> {
    let script = format!(
        "{TLS_FRONT}
        local http = require '@jess/http'
        local CA, SERVER_CERT, SERVER_KEY = {CA:?}, {SERVER_CERT:?}, {SERVER_KEY:?}
        local out: {{ string }} = {{}}
        local loop = net.loop()
        local task = loop:spawn(function()
          {body}
        end, 'test')
        local deadline = net.now() + 20000
        while task.status == 'running' and net.now() < deadline do
          loop:step(50)
        end
        local status, problem = task.status, task.error
        loop:close()
        if status ~= 'done' then error(`the test task ended {{status}}: {{problem}}`) end
        return table.concat(out, '\\n')
        "
    );
    let text = runtime.eval::<String>(&script).unwrap_or_else(|error| panic!("{error}"));
    text.lines().map(str::to_owned).collect()
}

const CLIENT: &str = r#"
    local udp = require("@dream/udp")
    local schema = udp.schema {
        version = 1,
        channels = {{ name = "reliable", delivery = "reliableOrdered" }},
        events = {{ name = "ping", channel = "reliable", maxPayload = 8 }},
    }
    return udp.client { schema = schema }
"#;

#[test]
fn transport_is_denied_by_default_and_requires_an_explicit_host_grant() {
    let denied = runtime(&[]);
    let error = denied.eval::<()>(CLIENT).unwrap_err().to_string();
    assert!(error.contains("network.transport"), "{error}");

    let granted = runtime(&["network.transport".to_owned()]);
    granted.eval::<()>(CLIENT).unwrap_or_else(|error| panic!("{error}"));
}

const LISTENER: &str = r#"
    local tcp = require("@dream/tcp")
    local listener = tcp.listen("127.0.0.1:0")
    listener:close()
"#;

const LOOKUP: &str = r#"
    local dns = require("@dream/dns")
    local request = assert(dns.resolve("localhost", 80))
    request:cancel()
"#;

#[test]
fn tcp_and_dns_are_denied_by_default_and_require_explicit_host_grants() {
    for (script, capability) in [(LISTENER, "network.tcp.listen"), (LOOKUP, "network.dns.resolve")] {
        let denied = runtime(&[]);
        let error = denied.eval::<()>(script).unwrap_err().to_string();
        assert!(error.contains(capability), "{error}");

        let granted = runtime(&[capability.to_owned()]);
        granted
            .eval::<()>(script)
            .unwrap_or_else(|error| panic!("{capability}: {error}"));
    }
}
