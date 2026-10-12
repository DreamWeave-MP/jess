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
