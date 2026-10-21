//! `@jess/http` on the wire, against a loopback HTTP/1.1 server: bodies by length, by chunks and
//! by close; redirects followed and bounded; sizes and time bounded; and nothing without the
//! runtime's grant.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write as _};
use std::net::{TcpListener, TcpStream};
use std::thread;

use jess::lua::Options;
use jess::lua::l3i::Runtime;

/// What the fixture answers to one path.
#[derive(Clone)]
enum Answer {
    /// A full response text, written as is.
    Raw(String),
    /// Accept, read the request, and never answer.
    Hang,
}

struct Server {
    port: u16,
}

impl Server {
    fn start(routes: Vec<(&'static str, Answer)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let routes = routes.clone();
                thread::spawn(move || answer(stream, &routes));
            }
        });
        Self { port }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

fn answer(mut stream: TcpStream, routes: &[(&str, Answer)]) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
            break;
        }
    }
    let answer = routes
        .iter()
        .find(|(route, _)| *route == path)
        .map_or_else(|| Answer::Raw(response(404, "not found", &[])), |(_, answer)| answer.clone());
    match answer {
        Answer::Raw(text) => {
            drop(stream.write_all(text.as_bytes()));
        }
        Answer::Hang => {
            let mut sink = [0; 1];
            drop(reader.read(&mut sink));
            thread::sleep(std::time::Duration::from_secs(5));
        }
    }
}

fn response(status: u16, body: &str, headers: &[(&str, &str)]) -> String {
    let mut text = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        let _ = write!(text, "{name}: {value}\r\n");
    }
    text.push_str("\r\n");
    text.push_str(body);
    text
}

fn runtime() -> Runtime {
    jess::lua::runtime_with_capabilities(
        Options::default(),
        &["network.dns.resolve".to_owned(), "network.tcp.connect".to_owned()],
    )
    .unwrap()
}

/// Fetches `url` with `options` (Luau table text) and returns `status, body, kind, message, final
/// url, redirect count` as the script reports them.
fn fetch(runtime: &Runtime, url: &str, options: &str) -> (Option<f64>, String, String, String, String, f64) {
    let script = format!(
        "
        local http = require('@jess/http')
        local response, message, kind = http.get({url:?}, {options})
        if not response then return nil, '', kind, message, '', 0 end
        return response.status, buffer.tostring(response.body), '', response.headers['content-type'] or '', response.url, #response.redirects
        "
    );
    runtime
        .eval::<(Option<f64>, String, String, String, String, f64)>(&script)
        .unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn a_body_arrives_by_length_by_chunks_and_by_close() {
    let server = Server::start(vec![
        ("/plain", Answer::Raw(response(200, "hello", &[("Content-Type", "text/plain")]))),
        (
            "/chunked",
            Answer::Raw(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"
                    .to_owned(),
            ),
        ),
        (
            "/until-close",
            Answer::Raw("HTTP/1.0 200 OK\r\nConnection: close\r\n\r\nall of it".to_owned()),
        ),
        ("/head", Answer::Raw(response(200, "ignored", &[]))),
    ]);
    let runtime = runtime();
    let (status, body, kind, content_type, url, redirects) = fetch(&runtime, &server.url("/plain"), "{}");
    assert_eq!((status, body.as_str(), kind.as_str()), (Some(200.0), "hello", ""));
    assert_eq!(content_type, "text/plain", "header names are lower-cased");
    assert_eq!((url, redirects), (server.url("/plain"), 0.0));

    let (status, body, ..) = fetch(&runtime, &server.url("/chunked"), "{}");
    assert_eq!((status, body.as_str()), (Some(200.0), "hello world"));

    let (status, body, ..) = fetch(&runtime, &server.url("/until-close"), "{}");
    assert_eq!((status, body.as_str()), (Some(200.0), "all of it"));

    let (status, body, ..) = fetch(&runtime, &server.url("/head"), "{ method = 'HEAD' }");
    assert_eq!((status, body.as_str()), (Some(200.0), ""));

    let (status, body, ..) = fetch(&runtime, &server.url("/missing"), "{}");
    assert_eq!(
        (status, body.as_str()),
        (Some(404.0), "not found"),
        "a status is a response, not an error"
    );
}

#[test]
fn redirects_are_followed_one_hop_at_a_time_and_bounded() {
    let server = Server::start(vec![
        ("/start", Answer::Raw(response(302, "", &[("Location", "/middle")]))),
        ("/middle", Answer::Raw(response(301, "", &[("Location", "end")]))),
        ("/end", Answer::Raw(response(200, "arrived", &[]))),
        ("/loop", Answer::Raw(response(307, "", &[("Location", "/loop")]))),
        (
            "/downgrade",
            Answer::Raw(response(302, "", &[("Location", "ftp://example.org/x")])),
        ),
    ]);
    let runtime = runtime();
    let (status, body, _, _, url, redirects) = fetch(&runtime, &server.url("/start"), "{}");
    assert_eq!((status, body.as_str()), (Some(200.0), "arrived"));
    assert_eq!((url, redirects), (server.url("/end"), 2.0));

    let (status, _, _, _, url, _) = fetch(&runtime, &server.url("/start"), "{ maxRedirects = 0 }");
    assert_eq!(
        (status, url),
        (Some(302.0), server.url("/start")),
        "zero hops returns the redirect itself"
    );

    let (status, _, kind, message, ..) = fetch(&runtime, &server.url("/loop"), "{}");
    assert_eq!((status, kind.as_str()), (None, "tooManyRedirects"), "{message}");

    let (status, _, kind, ..) = fetch(&runtime, &server.url("/downgrade"), "{}");
    assert_eq!((status, kind.as_str()), (None, "unsupportedScheme"));
}

#[test]
fn sizes_and_time_are_bounded_and_failures_are_named() {
    let server = Server::start(vec![
        ("/big", Answer::Raw(response(200, &"x".repeat(5000), &[]))),
        (
            "/big-until-close",
            Answer::Raw(format!("HTTP/1.0 200 OK\r\n\r\n{}", "y".repeat(5000))),
        ),
        ("/slow", Answer::Hang),
        ("/garbage", Answer::Raw("not http at all\r\n\r\n".to_owned())),
    ]);
    let runtime = runtime();
    let (status, _, kind, message, ..) = fetch(&runtime, &server.url("/big"), "{ maxBytes = 1000 }");
    assert_eq!((status, kind.as_str()), (None, "tooLarge"), "{message}");
    assert!(message.contains("5000"), "{message}");

    let (status, _, kind, ..) = fetch(&runtime, &server.url("/big-until-close"), "{ maxBytes = 1000 }");
    assert_eq!((status, kind.as_str()), (None, "tooLarge"));

    let started = std::time::Instant::now();
    let (status, _, kind, ..) = fetch(&runtime, &server.url("/slow"), "{ timeoutMs = 400 }");
    assert_eq!((status, kind.as_str()), (None, "timeout"));
    assert!(started.elapsed() < std::time::Duration::from_secs(3));

    let (status, _, kind, ..) = fetch(&runtime, &server.url("/garbage"), "{}");
    assert_eq!((status, kind.as_str()), (None, "protocol"));

    let (status, _, kind, ..) = fetch(&runtime, "http://127.0.0.1:1/nothing-listens", "{ timeoutMs = 2000 }");
    assert_eq!((status, kind.as_str()), (None, "connect"));

    let (status, _, kind, ..) = fetch(&runtime, "https://user:pw@example.org/", "{}");
    assert_eq!((status, kind.as_str()), (None, "credentialsInUrl"));
}

#[test]
fn nothing_is_fetched_without_the_runtime_s_grant() {
    let server = Server::start(vec![("/plain", Answer::Raw(response(200, "hello", &[])))]);
    let denied = jess::lua::runtime_with_capabilities(Options::default(), &[]).unwrap();
    let script = format!(
        "local http = require('@jess/http')\nlocal response, message = http.get({:?}, {{}})\nreturn response == nil, message",
        server.url("/plain")
    );
    let (failed, message) = denied.eval::<(bool, String)>(&script).unwrap_or_else(|error| {
        // Reaching the socket raised the denial instead of returning it; either is a refusal.
        let text = error.to_string();
        assert!(text.contains("network."), "{text}");
        (true, text)
    });
    assert!(failed, "{message}");
    assert!(message.contains("network."), "{message}");
}
