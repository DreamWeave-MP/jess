//! `@jess/http` on the wire, against a loopback HTTP/1.1 server: bodies by length, by chunks and
//! by close; any method with headers and a body; redirects followed, bounded, and never carrying
//! credentials to another origin; sizes and time bounded; requests in tasks that leave the rest of
//! the loop running; HTTPS through the Luau TLS front; and nothing without the runtime's grant.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write as _};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use jess::lua::Options;
use jess::lua::l3i::Runtime;

use super::{run_task, tls_runtime};

/// What the fixture answers to one path.
#[derive(Clone)]
pub(super) enum Answer {
    /// A full response text, written as is.
    Raw(String),
    /// Accept, read the request, and never answer.
    Hang,
    /// The request as it arrived (head, then body), as the response body.
    Echo,
    /// A full response text, written after a pause.
    Slow(Duration, String),
}

pub(super) struct Server {
    port: u16,
}

impl Server {
    pub(super) fn start(routes: Vec<(&'static str, Answer)>) -> Self {
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

    pub(super) fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

fn answer(mut stream: TcpStream, routes: &[(&str, Answer)]) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut head = String::new();
    if reader.read_line(&mut head).is_err() {
        return;
    }
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let path = path.split('?').next().unwrap_or("/").to_owned();
    let mut length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.is_empty() {
            break;
        }
        head.push_str(&line);
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
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
        Answer::Echo => {
            let mut body = vec![0; length];
            if reader.read_exact(&mut body).is_err() {
                return;
            }
            let echoed = format!("{head}{}", String::from_utf8_lossy(&body));
            drop(stream.write_all(response(200, &echoed, &[]).as_bytes()));
        }
        Answer::Slow(pause, text) => {
            thread::sleep(pause);
            drop(stream.write_all(text.as_bytes()));
        }
        Answer::Hang => {
            let mut sink = [0; 1];
            drop(reader.read(&mut sink));
            thread::sleep(Duration::from_secs(5));
        }
    }
}

pub(super) fn response(status: u16, body: &str, headers: &[(&str, &str)]) -> String {
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
    assert!(started.elapsed() < Duration::from_secs(3));

    let (status, _, kind, ..) = fetch(&runtime, &server.url("/garbage"), "{}");
    assert_eq!((status, kind.as_str()), (None, "protocol"));

    // Windows retries a refused SYN a few times, a second apart, before it reports the refusal, so
    // the timeout stays well clear of that or the failure would be named timeout instead.
    let (status, _, kind, ..) = fetch(&runtime, "http://127.0.0.1:1/nothing-listens", "{ timeoutMs = 10000 }");
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

/// Evaluates `body` (Luau that returns one string) outside any task, where requests run on a loop
/// of their own.
fn eval(runtime: &Runtime, body: &str) -> String {
    let script = format!("local http = require('@jess/http')\n{body}");
    runtime.eval::<String>(&script).unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn any_method_goes_out_with_its_headers_and_body() {
    let server = Server::start(vec![("/echo", Answer::Echo)]);
    let echoed = eval(
        &runtime(),
        &format!(
            "
            local lines = {{}}
            local function send(request)
              local response, message = http.request(request)
              assert(response, message)
              table.insert(lines, buffer.tostring(response.body))
            end

            send {{ url = {url:?}, method = 'POST', json = {{ content = 'hi', n = 1 }}, headers = {{ Authorization = 'Bot secret', ['X-Trace'] = 'a' }} }}
            send {{ url = {url:?}, method = 'PUT', body = buffer.fromstring('raw bytes'), headers = {{ ['Content-Type'] = 'application/octet-stream' }} }}
            send {{ url = {url:?}, method = 'PATCH', body = 'patched' }}
            send {{ url = {url:?}, method = 'DELETE' }}
            send {{ url = {url:?}, method = 'POST' }}
            send {{ url = {url:?}, method = 'PURGE' }}
            return table.concat(lines, '\\n=====\\n')
            ",
            url = server.url("/echo")
        ),
    );
    let requests: Vec<&str> = echoed.split("\n=====\n").collect();
    let expect = |request: &str, start: &str, lines: &[&str], body: &str| {
        assert!(request.starts_with(start), "{request}");
        for line in lines {
            assert!(request.contains(&format!("{line}\r\n")), "{line} in {request}");
        }
        assert!(request.ends_with(&format!("\r\n\r\n{body}")), "{request}");
    };
    expect(
        requests[0],
        "POST /echo HTTP/1.1\r\n",
        &[
            "authorization: Bot secret",
            "content-type: application/json",
            "content-length: 22",
            "connection: close",
            "x-trace: a",
        ],
        r#"{"content":"hi","n":1}"#,
    );
    assert!(
        requests[0].find("authorization").unwrap() < requests[0].find("x-trace").unwrap(),
        "fields go out in name order"
    );
    expect(
        requests[1],
        "PUT /echo HTTP/1.1\r\n",
        &["content-type: application/octet-stream", "content-length: 9"],
        "raw bytes",
    );
    expect(requests[2], "PATCH /echo HTTP/1.1\r\n", &["content-length: 7"], "patched");
    assert!(requests[3].starts_with("DELETE /echo HTTP/1.1\r\n") && !requests[3].contains("content-length"));
    expect(requests[4], "POST /echo HTTP/1.1\r\n", &["content-length: 0"], "");
    assert!(requests[5].starts_with("PURGE /echo HTTP/1.1\r\n"));
}

#[test]
fn a_mistaken_request_raises_before_anything_is_sent() {
    let runtime = runtime();
    for (request, expected) in [
        (
            "{ url = 'http://127.0.0.1:1/', headers = { Host = 'elsewhere' } }",
            "written by the client",
        ),
        (
            "{ url = 'http://127.0.0.1:1/', headers = { ['X-Bad'] = 'a\\r\\nInjected: yes' } }",
            "without line breaks",
        ),
        (
            "{ url = 'http://127.0.0.1:1/', headers = { ['Bad Name'] = 'x' } }",
            "not a header field name",
        ),
        ("{ url = 'http://127.0.0.1:1/', method = 'GET /x' }", "not an HTTP method"),
        (
            "{ url = 'http://127.0.0.1:1/', body = 'a', json = {} }",
            "body or json, not both",
        ),
        (
            "{ url = 'http://127.0.0.1:1/', bodyy = 'a' }",
            "unknown jess.http.request option 'bodyy'",
        ),
    ] {
        let error = runtime
            .eval::<()>(&format!("require('@jess/http').request({request})"))
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{request}: {error}");
        assert!(!error.contains("Injected"), "a header value never reaches a message: {error}");
    }
}

#[test]
fn redirects_keep_or_drop_the_method_and_never_carry_credentials_to_another_origin() {
    let other = Server::start(vec![("/echo", Answer::Echo)]);
    let server = Server::start(vec![
        ("/echo", Answer::Echo),
        ("/same", Answer::Raw(response(307, "", &[("Location", "/echo")]))),
        ("/see-other", Answer::Raw(response(303, "", &[("Location", "/echo")]))),
        ("/found", Answer::Raw(response(302, "", &[("Location", "/echo")]))),
        ("/away", Answer::Raw(response(308, "", &[("Location", &other.url("/echo"))]))),
    ]);
    let echoed = eval(
        &runtime(),
        &format!(
            "
            local lines = {{}}
            for _, path in {{ '/same', '/see-other', '/found', '/away' }} do
              local response, message = http.request {{
                url = {base:?} .. path,
                method = 'POST',
                body = 'payload',
                headers = {{ Authorization = 'Bearer token', Cookie = 'session=1', ['X-Kept'] = 'yes' }},
              }}
              assert(response, message)
              table.insert(lines, buffer.tostring(response.body))
            end
            return table.concat(lines, '\\n=====\\n')
            ",
            base = server.url("")
        ),
    );
    let requests: Vec<&str> = echoed.split("\n=====\n").collect();
    assert!(
        requests[0].starts_with("POST /echo") && requests[0].ends_with("payload"),
        "307 repeats the request"
    );
    assert!(
        requests[0].contains("authorization: Bearer token"),
        "the same origin keeps credentials"
    );
    assert!(
        requests[1].starts_with("GET /echo") && !requests[1].ends_with("payload"),
        "303 goes on as GET"
    );
    assert!(requests[2].starts_with("GET /echo"), "302 after a POST goes on as GET");
    assert!(
        requests[3].starts_with("POST /echo") && requests[3].ends_with("payload"),
        "308 repeats it"
    );
    assert!(
        !requests[3].contains("authorization") && !requests[3].contains("cookie"),
        "{}",
        requests[3]
    );
    assert!(requests[3].contains("x-kept: yes"), "other fields still go");
}

#[test]
fn framing_errors_interim_answers_and_secrets_in_urls() {
    let server = Server::start(vec![
        (
            "/interim",
            Answer::Raw(format!("HTTP/1.1 100 Continue\r\n\r\n{}", response(200, "final", &[]))),
        ),
        (
            "/disagrees",
            Answer::Raw("HTTP/1.1 200 OK\r\nContent-Length: 5, 6\r\n\r\nhello!".to_owned()),
        ),
        (
            "/bad-chunk",
            Answer::Raw("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nhello\r\n0\r\n\r\n".to_owned()),
        ),
        (
            "/chunks-and-length",
            Answer::Raw(
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\nX-Trailer: t\r\n\r\n"
                    .to_owned(),
            ),
        ),
        (
            "/big-chunks",
            Answer::Raw(format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n400\r\n{}\r\n400\r\n{}\r\n0\r\n\r\n",
                "a".repeat(1024),
                "b".repeat(1024)
            )),
        ),
        ("/folded", Answer::Raw("HTTP/1.1 200 OK\r\nX-A: one\r\n two\r\n\r\n".to_owned())),
        ("/json", Answer::Raw(response(200, r#"{"ok":true,"items":[1,2]}"#, &[]))),
        ("/slow", Answer::Hang),
    ]);
    let runtime = runtime();
    let (status, body, ..) = fetch(&runtime, &server.url("/interim"), "{}");
    assert_eq!((status, body.as_str()), (Some(200.0), "final"), "a 100 answer is skipped");
    let (status, _, kind, ..) = fetch(&runtime, &server.url("/disagrees"), "{}");
    assert_eq!((status, kind.as_str()), (None, "protocol"));
    let (status, _, kind, ..) = fetch(&runtime, &server.url("/bad-chunk"), "{}");
    assert_eq!((status, kind.as_str()), (None, "protocol"));
    let (status, body, ..) = fetch(&runtime, &server.url("/chunks-and-length"), "{}");
    assert_eq!(
        (status, body.as_str()),
        (Some(200.0), "abc"),
        "chunked wins over Content-Length"
    );
    let (status, _, kind, ..) = fetch(&runtime, &server.url("/big-chunks"), "{ maxBytes = 1500 }");
    assert_eq!((status, kind.as_str()), (None, "tooLarge"));
    let (status, body, ..) = fetch(&runtime, &server.url("/big-chunks"), "{ maxBytes = 2048 }");
    assert_eq!((status, body.len()), (Some(200.0), 2048));
    let (status, _, kind, ..) = fetch(&runtime, &server.url("/folded"), "{}");
    assert_eq!((status, kind.as_str()), (None, "protocol"));

    let decoded = eval(
        &runtime,
        &format!(
            "local response = assert(http.get({url:?}))
            local value = http.json(response) :: {{ ok: boolean, items: {{ number }} }}
            local _, problem = http.json({{ body = buffer.fromstring('not json') }} :: any)
            return `{{value.ok}} {{#value.items}} {{problem ~= nil}}`",
            url = server.url("/json")
        ),
    );
    assert_eq!(decoded, "true 2 true");

    let (_, _, kind, message, ..) = fetch(&runtime, &(server.url("/slow") + "?token=SECRET"), "{ timeoutMs = 300 }");
    assert_eq!(kind, "timeout");
    assert!(!message.contains("SECRET"), "a query never reaches a message: {message}");
}

#[test]
fn a_slow_request_in_a_task_leaves_the_rest_of_the_loop_running() {
    let server = Server::start(vec![
        (
            "/slow",
            Answer::Slow(Duration::from_millis(1500), response(200, "eventually", &[])),
        ),
        ("/fast", Answer::Raw(response(200, "at once", &[]))),
    ]);
    let lines = run_task(
        &runtime(),
        &format!(
            "
            local started = net.now()
            local ticks, longestGap, last = 0, 0, net.now()
            local done = false
            local ticker = loop:spawn(function()
              while not done do
                net.sleep(20)
                local current = net.now()
                longestGap = math.max(longestGap, current - last)
                last, ticks = current, ticks + 1
              end
            end, 'ticker')

            local fastAt
            local fast = loop:spawn(function()
              local response = assert(http.get({fast:?}))
              fastAt = net.now() - started
              table.insert(out, `fast {{buffer.tostring(response.body)}}`)
            end, 'fast')

            local response = assert(http.get({slow:?}, {{ timeoutMs = 10000 }}))
            done = true
            ticker:join()
            fast:join()
            table.insert(out, `slow {{buffer.tostring(response.body)}}`)
            table.insert(out, `fast first {{fastAt < 1000}}`)
            table.insert(out, `ticked {{ticks >= 30}} gaps {{longestGap < 300}}`)
            ",
            slow = server.url("/slow"),
            fast = server.url("/fast"),
        ),
    );
    assert_eq!(
        lines,
        ["fast at once", "slow eventually", "fast first true", "ticked true gaps true"]
    );
}

#[test]
fn https_is_verified_against_the_roots_it_is_given() {
    let server = Server::start(vec![
        (
            "/chunked",
            Answer::Raw(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"
                    .to_owned(),
            ),
        ),
        ("/echo", Answer::Echo),
    ]);
    let lines = run_task(
        &tls_runtime(),
        &format!(
            "
            local port = tlsFront(loop, {port}, SERVER_CERT, SERVER_KEY)
            local tls = require '@dream/tls'
            local trusted = assert(tls.clientConfig {{ roots = CA, platform = false }})
            local response = assert(http.get(`https://localhost:{{port}}/chunked`, {{ tls = trusted }}))
            table.insert(out, `{{response.status}} {{buffer.tostring(response.body)}}`)

            local posted = assert(http.request {{
              url = `https://127.0.0.1:{{port}}/echo`,
              method = 'POST',
              json = {{ model = 'test' }},
              headers = {{ Authorization = 'Bearer key' }},
              tls = trusted,
            }})
            local echoed = buffer.tostring(posted.body)
            table.insert(out, `post {{string.find(echoed, 'authorization: Bearer key', 1, true) ~= nil}} {{string.sub(echoed, -16)}}`)

            local _, problem, kind = http.get(`https://localhost:{{port}}/chunked`)
            table.insert(out, `platform trust {{kind}} {{string.find(problem, 'Untrusted') ~= nil or string.find(problem, 'trust') ~= nil}}`)

            local connection, mismatch, mismatchKind = net.connect('127.0.0.1', port, {{
              tls = true, serverName = 'example.org', tlsConfig = trusted, deadline = net.now() + 5000,
            }})
            table.insert(out, `wrong name {{connection == nil}} {{mismatchKind}}`)
            ",
            port = server.port
        ),
    );
    assert_eq!(lines[0], "200 hello world");
    assert_eq!(lines[1], r#"post true {"model":"test"}"#);
    assert!(lines[2].starts_with("platform trust tls"), "{lines:?}");
    assert_eq!(lines[3], "wrong name true tls");
}

#[test]
fn a_url_with_a_line_break_fails_before_anything_connects() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let runtime = runtime();
    for url in [
        format!("http://127.0.0.1:{port}/a\r\nX-Injected: yes"),
        format!("http://127.0.0.1:{port}/a b HTTP/1.1"),
        format!("http://127.0.0.1:{port}\r\n/"),
    ] {
        let (status, _, kind, message, ..) = fetch(&runtime, &url, "{}");
        assert_eq!((status, kind.as_str()), (None, "invalidUrl"), "{message}");
        assert!(!message.contains('\n'), "{message}");
    }
    assert_eq!(
        listener.accept().map(|_| ()).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "nothing connected"
    );
}
