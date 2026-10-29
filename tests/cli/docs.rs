//! Executable documentation examples that need a loopback service.

use super::common::{http::Server, jess};

#[test]
fn the_http_tutorial_fetches_a_loopback_resource() {
    let server = Server::start();
    server.serve("/message", "text/plain", "hello from the fixture");
    let output = jess()
        .args([
            "run",
            "--capability",
            "network.dns.resolve",
            "--capability",
            "network.tcp.connect",
            "tests/fixtures/docs/http-get.jsl",
            &server.url("/message"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hello from the fixture\n");
    assert_eq!(server.requests(), ["/message"]);
}
