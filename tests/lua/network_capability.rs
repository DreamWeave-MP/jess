use jess::lua::Options;
use jess::lua::l3i::Runtime;

fn runtime(capabilities: &[String]) -> Runtime {
    jess::lua::runtime_with_capabilities(Options::default(), capabilities).unwrap()
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
