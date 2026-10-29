//! `http.serve` on the wire: a server running in a runtime of its own on a thread, and raw sockets
//! from here sending exactly the bytes each test wants, well-formed or not; then jess's own client
//! against jess's own server on one loop, over TCP and TLS, for keep-alive and event streams.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use jess::lua::Options;

use super::{run_task, tls_runtime};

/// The handler every raw test talks to, and the server around it, run until `/quit`; what the
/// server reported through `onError` comes back, one line each.
const SERVER: &str = r"
local http = require '@jess/http'
local net = require '@jess/net'
local port = ...
local errors: { string } = {}

local function handle(request: http.ServerRequest): http.ServerResponse
  if request.path == '/hello' then return { headers = { ['Content-Type'] = 'text/plain' }, body = 'hi' } end
  if request.path == '/json' then return { json = { ok = true } } end
  if request.path == '/fail' then error('boom', 0) end
  if request.path == '/bad-header' then return { headers = { ['X-Bad'] = 'a\r\nInjected: 1' }, body = 'no' } end
  if request.path == '/bad-json' then return { json = { value = 0 / 0 } } end

  if request.path == '/stream-fail' then
    return {
      stream = function(send: (data: string | buffer) -> boolean)
        send('part')
        error('stream broke', 0)
      end,
    }
  end

  if request.path == '/echo' then
    local kind = request.headers['content-type'] or ''
    return { body = `{request.method} {request.path}?{request.query} {kind} {buffer.len(request.body)}|{buffer.tostring(request.body)}` }
  end

  if request.path == '/slow' then
    net.sleep(300)
    return { body = 'slow' }
  end

  if request.path == '/stream' then
    return {
      headers = { ['Content-Type'] = 'text/plain' },
      stream = function(send: (data: string | buffer) -> boolean)
        send('one ')
        net.sleep(150)
        send('two ')
        net.sleep(150)
        send('three')
      end,
    }
  end

  return { status = 404, body = 'missing' }
end

local quit = net.signal()
local finished, problem = net.run(function()
  local server = assert(http.serve {
    address = `127.0.0.1:{port}`,
    maxConnections = 4,
    maxHeadBytes = 2048,
    maxBodyBytes = 1000,
    headTimeoutMs = 400,
    handler = function(request: http.ServerRequest): http.ServerResponse
      if request.path == '/quit' then
        quit:notify()
        return { body = 'bye' }
      end

      return handle(request)
    end,
    onError = function(message: string) table.insert(errors, message) end,
  })

  quit:wait()
  net.sleep(50)
  server:close()
end)

assert(finished, problem)
return table.concat(errors, '\n')
";

/// A server from `SERVER` on a free port, running until dropped.
struct Served {
    port: u16,
    done: Option<thread::JoinHandle<String>>,
}

impl Served {
    fn start() -> Self {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let done = thread::spawn(move || {
            let runtime =
                jess::lua::runtime_with_capabilities(Options::default(), &["network.tcp.listen".to_owned()]).unwrap();
            let script = format!("return (function(...) {SERVER} end)({port})");
            runtime.eval::<String>(&script).unwrap_or_else(|error| panic!("{error}"))
        });
        let started = Instant::now();
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(started.elapsed() < Duration::from_secs(10), "the server never listened");
            thread::sleep(Duration::from_millis(10));
        }
        Self { port, done: Some(done) }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream
    }

    /// Asks the server to stop and returns what it reported.
    fn finish(mut self) -> String {
        let mut stream = self.connect();
        stream
            .write_all(b"GET /quit HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
            .unwrap();
        drop(read_all(&mut stream));
        self.done.take().unwrap().join().unwrap()
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        if self.done.is_some()
            && let Ok(mut stream) = TcpStream::connect(("127.0.0.1", self.port))
        {
            drop(stream.write_all(b"GET /quit HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n"));
        }
    }
}

fn read_all(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    drop(stream.read_to_end(&mut bytes));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// One response off `stream`: its head, and its body by Content-Length.
fn read_response(stream: &mut TcpStream) -> (String, String) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            break;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.trim().parse().unwrap());
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).unwrap();
    (head, String::from_utf8_lossy(&body).into_owned())
}

/// What the server answers to `request` sent whole, read until it closes.
fn exchange(server: &Served, request: &str) -> String {
    let mut stream = server.connect();
    stream.write_all(request.as_bytes()).unwrap();
    read_all(&mut stream)
}

#[test]
fn requests_are_answered_kept_alive_and_closed_when_asked() {
    let server = Served::start();
    let mut stream = server.connect();
    stream.write_all(b"GET /hello HTTP/1.1\r\nHost: test\r\n\r\n").unwrap();
    let (head, body) = read_response(&mut stream);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert!(
        head.contains("\r\ndate: ") && head.contains("content-type: text/plain\r\n"),
        "{head}"
    );
    assert!(!head.contains("connection: close"), "{head}");
    assert_eq!(body, "hi");

    stream.write_all(b"GET /json HTTP/1.1\r\nHost: test\r\n\r\n").unwrap();
    let (head, body) = read_response(&mut stream);
    assert!(head.contains("content-type: application/json\r\n"), "{head}");
    assert_eq!(body, r#"{"ok":true}"#, "the second request rode the same connection");

    stream
        .write_all(b"GET /hello HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
        .unwrap();
    let (head, _) = read_response(&mut stream);
    assert!(head.contains("connection: close\r\n"), "{head}");
    assert_eq!(read_all(&mut stream), "", "and then the server closed it");

    let old = exchange(&server, "GET /hello HTTP/1.0\r\n\r\n");
    assert!(
        old.starts_with("HTTP/1.1 200 OK") && old.contains("connection: close"),
        "HTTP/1.0 closes: {old}"
    );
    assert_eq!(server.finish(), "");
}

#[test]
fn pipelined_requests_are_answered_in_order() {
    let server = Served::start();
    let mut stream = server.connect();
    stream
        .write_all(b"\r\nGET /json HTTP/1.1\r\nHost: test\r\n\r\nGET /hello HTTP/1.1\r\nHost: test\r\n\r\n")
        .unwrap();
    let (_, first) = read_response(&mut stream);
    let (_, second) = read_response(&mut stream);
    assert_eq!((first.as_str(), second.as_str()), (r#"{"ok":true}"#, "hi"));
    drop(server.finish());
}

#[test]
fn bodies_arrive_by_length_and_by_chunks_and_head_sends_none() {
    let server = Served::start();
    let mut stream = server.connect();
    stream
        .write_all(b"POST /echo?x=1 HTTP/1.1\r\nHost: test\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello")
        .unwrap();
    assert_eq!(read_response(&mut stream).1, "POST /echo?x=1 text/plain 5|hello");

    stream
        .write_all(b"PUT /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n3;note=\"x\"\r\nabc\r\n2\r\nde\r\n0\r\nTrailer: t\r\n\r\n")
        .unwrap();
    assert_eq!(read_response(&mut stream).1, "PUT /echo?  5|abcde");

    stream.write_all(b"HEAD /hello HTTP/1.1\r\nHost: test\r\n\r\n").unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    assert!(String::from_utf8_lossy(&head).contains("content-length: 2\r\n"));
    stream.write_all(b"GET /json HTTP/1.1\r\nHost: test\r\n\r\n").unwrap();
    assert_eq!(
        read_response(&mut stream).1,
        r#"{"ok":true}"#,
        "HEAD sent no body before this"
    );

    stream
        .write_all(b"POST /echo HTTP/1.1\r\nHost: test\r\nExpect: 100-continue\r\nContent-Length: 2\r\n\r\n")
        .unwrap();
    let mut interim = [0u8; 25];
    stream.read_exact(&mut interim).unwrap();
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    stream.write_all(b"ok").unwrap();
    assert_eq!(read_response(&mut stream).1, "POST /echo?  2|ok");
    drop(server.finish());
}

#[test]
fn requests_that_could_be_smuggled_or_are_malformed_are_refused_and_closed() {
    let server = Served::start();
    let cases = [
        (
            "POST /echo HTTP/1.1\r\nHost: t\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            "HTTP/1.1 400",
        ),
        (
            "POST /echo HTTP/1.1\r\nHost: t\r\nTransfer-Encoding: gzip, chunked\r\n\r\n",
            "HTTP/1.1 501",
        ),
        (
            "POST /echo HTTP/1.1\r\nHost: t\r\nContent-Length: 5, 6\r\n\r\nhello!",
            "HTTP/1.1 400",
        ),
        ("GET /a b HTTP/1.1\r\nHost: t\r\n\r\n", "HTTP/1.1 400"),
        ("GET /hello HTTP/1.1\r\n\r\n", "HTTP/1.1 400"),
        ("GET /hello HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n", "HTTP/1.1 400"),
        ("GET /hello HTTP/2.0\r\nHost: t\r\n\r\n", "HTTP/1.1 505"),
        ("GET /hello HTTP/1.1\r\nHost: t\r\nX-Bad: a\u{1}b\r\n\r\n", "HTTP/1.1 400"),
        ("GET /hello HTTP/1.1\r\nHost: t\r\n Folded: yes\r\n\r\n", "HTTP/1.1 400"),
        ("GET /hello HTTP/1.1\r\nHost: t\r\nExpect: tea\r\n\r\n", "HTTP/1.1 417"),
        (
            "POST /echo HTTP/1.1\r\nHost: t\r\nContent-Length: 5000\r\n\r\n",
            "HTTP/1.1 413",
        ),
        (
            "POST /echo HTTP/1.1\r\nHost: t\r\nTransfer-Encoding: chunked\r\n\r\n7d0\r\n",
            "HTTP/1.1 413",
        ),
        ("GET * HTTP/1.1\r\nHost: t\r\n\r\n", "HTTP/1.1 400"),
    ];
    for (request, expected) in cases {
        let answer = exchange(&server, request);
        assert!(answer.starts_with(expected), "{request:?}: {answer}");
        assert!(answer.contains("connection: close\r\n"), "{request:?}: {answer}");
    }
    let big = format!("GET /hello HTTP/1.1\r\nHost: t\r\nX-Big: {}\r\n\r\n", "x".repeat(3000));
    assert!(exchange(&server, &big).starts_with("HTTP/1.1 431"));
    assert_eq!(server.finish(), "");
}

#[test]
fn a_head_that_never_ends_is_dropped_without_an_answer() {
    let server = Served::start();
    let mut stream = server.connect();
    stream.write_all(b"GET /hello HTTP/1.1\r\nHost: t\r\n").unwrap();
    let started = Instant::now();
    let answer = read_all(&mut stream);
    assert_eq!(answer, "", "a half-sent head gets nothing");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(300) && waited < Duration::from_secs(3),
        "{waited:?}"
    );
    drop(server.finish());
}

#[test]
fn a_handler_that_raises_or_answers_wrongly_is_a_500_and_the_server_carries_on() {
    let server = Served::start();
    for path in ["/fail", "/bad-header"] {
        let answer = exchange(&server, &format!("GET {path} HTTP/1.1\r\nHost: t\r\n\r\n"));
        assert!(answer.starts_with("HTTP/1.1 500"), "{path}: {answer}");
        assert!(!answer.contains("Injected"), "{answer}");
    }
    assert!(exchange(&server, "GET /hello HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n").ends_with("hi"));
    let reported = server.finish();
    assert!(reported.contains("boom"), "{reported}");
    assert!(reported.contains("needs a string value without line breaks"), "{reported}");
}

#[test]
fn a_response_that_fails_to_encode_or_a_stream_that_raises_gives_its_connection_back() {
    let server = Served::start();
    let answer = exchange(&server, "GET /bad-json HTTP/1.1\r\nHost: t\r\n\r\n");
    assert!(answer.starts_with("HTTP/1.1 500"), "{answer}");

    // More failed streams than the server has places: each one's place must come back.
    for _ in 0..6 {
        let answer = exchange(&server, "GET /stream-fail HTTP/1.1\r\nHost: t\r\n\r\n");
        assert!(answer.contains("4\r\npart\r\n"), "{answer}");
        assert!(
            !answer.ends_with("0\r\n\r\n"),
            "a stream that raised isn't ended as if whole: {answer}"
        );
    }
    assert!(exchange(&server, "GET /hello HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n").ends_with("hi"));
    let reported = server.finish();
    assert!(reported.contains("NaN or infinite"), "{reported}");
    assert_eq!(reported.matches("stream broke").count(), 6, "{reported}");
}

#[test]
fn a_raising_on_error_still_gives_the_connection_back_and_fails_only_its_task() {
    let runtime = tls_runtime();
    let text = runtime
        .eval::<String>(
            "
            local http = require '@jess/http'
            local net = require '@jess/net'
            local lines: { string } = {}
            local failures: { string } = {}
            local loop = net.loop { onError = function(_task, message) table.insert(failures, message) end }
            local task = loop:spawn(function()
              local server = assert(http.serve {
                address = '127.0.0.1:0',
                maxConnections = 1,
                handler = function(request)
                  if request.path == '/fail' then error('handler broke', 0) end
                  return { body = 'fine' }
                end,
                onError = function(message) error(`onError broke on {message}`, 0) end,
              })
              for _ = 1, 3 do
                local response = assert(http.request { url = `http://127.0.0.1:{server.port}/fail` })
                table.insert(lines, tostring(response.status))
              end
              local response = assert(http.request { url = `http://127.0.0.1:{server.port}/fine`, timeoutMs = 3000 })
              table.insert(lines, buffer.tostring(response.body))
              server:close()
            end)
            loop:run(10000)
            loop:close()
            table.insert(lines, `{task.status} {#failures} {failures[1]}`)
            return table.concat(lines, '\\n')
            ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        ["500", "500", "500", "fine", "done 3 onError broke on handler broke"]
    );
}

#[test]
fn failed_and_abandoned_tls_handshakes_give_their_places_back() {
    let lines = run_task(
        &tls_runtime(),
        "
        local tls = require '@dream/tls'
        local config = assert(tls.serverConfig { certChain = SERVER_CERT, privateKey = SERVER_KEY })
        local server = assert(http.serve {
          address = '127.0.0.1:0',
          tls = config,
          maxConnections = 2,
          headTimeoutMs = 300,
          handler = function() return { body = 'fine' } end,
        })

        -- More refused handshakes than the server has places.
        for _ = 1, 6 do
          local raw = assert(net.connect('127.0.0.1', server.port))
          assert(raw:send('this is not a TLS client hello\\r\\n\\r\\n'))
          while raw:receiveSome(4096, net.now() + 3000) do end
          raw:close()
        end

        -- And one that says nothing until its handshake times out.
        local silent = assert(net.connect('127.0.0.1', server.port))
        while silent:receiveSome(4096, net.now() + 3000) do end
        silent:close()
        table.insert(out, `after failures {server:stats().connections}`)

        local trusted = assert(tls.clientConfig { roots = CA, platform = false })
        local response = assert(http.request { url = `https://localhost:{server.port}/`, tls = trusted, timeoutMs = 3000 })
        table.insert(out, buffer.tostring(response.body))

        -- A handshake still waiting when the server closes is closed with it.
        local pending = assert(net.connect('127.0.0.1', server.port))
        net.sleep(50)
        table.insert(out, `pending {server:stats().connections}`)
        server:close()
        table.insert(out, `closed {server:stats().connections}`)
        table.insert(out, `pending ended {pending:receiveSome(10, net.now() + 2000) == nil}`)
        pending:close()
        ",
    );
    assert_eq!(
        lines,
        ["after failures 0", "fine", "pending 1", "closed 0", "pending ended true"]
    );
}

#[test]
fn a_closed_server_frees_its_address_at_once_before_during_and_after_accepting() {
    let lines = run_task(
        &tls_runtime(),
        "
        local function handle() return { body = 'ok' } end
        local first = assert(http.serve { address = '127.0.0.1:0', handler = handle })
        local port = first.port
        -- Before its acceptor has run at all.
        first:close()

        local second = assert(http.serve { address = `127.0.0.1:{port}`, handler = handle })
        net.sleep(30)
        -- While its acceptor waits on the listener.
        second:close()

        local third = assert(http.serve { address = `127.0.0.1:{port}`, handler = handle })
        local response = assert(http.request { url = `http://127.0.0.1:{port}/` })
        table.insert(out, buffer.tostring(response.body))
        third:close()
        net.sleep(30)
        -- Once its acceptor has finished, closing again does nothing.
        third:close()
        local refused = net.connect('127.0.0.1', port, { deadline = net.now() + 1000 })
        table.insert(out, `refused {refused == nil}`)
        ",
    );
    assert_eq!(lines, ["ok", "refused true"]);
}

#[test]
fn a_streamed_write_the_client_never_takes_ends_at_the_write_timeout() {
    let lines = run_task(
        &tls_runtime(),
        "
        local reports: { string } = {}
        local ended = net.signal()
        local outcome = ''
        local server = assert(http.serve {
          address = '127.0.0.1:0',
          writeTimeoutMs = 200,
          handler = function()
            return {
              stream = function(send)
                -- More than any platform buffers for a client that reads nothing.
                local chunk = string.rep('x', 4 * 1024 * 1024)
                local count = 0
                while count < 64 and send(chunk) do
                  count += 1
                end
                outcome = `stopped {count < 64}`
                ended:notify()
              end,
            }
          end,
          onError = function(message) table.insert(reports, message) end,
        })

        local raw = assert(net.connect('127.0.0.1', server.port))
        assert(raw:send('GET /forever HTTP/1.1\\r\\nHost: t\\r\\n\\r\\n'))
        table.insert(out, `notified {ended:wait(net.now() + 15000)}`)
        net.sleep(20)
        table.insert(out, outcome)
        local timedOut = string.find(reports[1] or '', 'longer than the time allowed', 1, true) ~= nil
        table.insert(out, `reported {#reports} {timedOut}`)
        table.insert(out, `connections {server:stats().connections}`)
        raw:close()
        server:close()
        ",
    );
    assert_eq!(lines, ["notified true", "stopped true", "reported 1 true", "connections 0"]);
}

#[test]
fn bad_options_are_refused_before_anything_listens() {
    for option in [
        "maxHeadBytes = -1",
        "writeTimeoutMs = 0 / 0",
        "bodyTimeoutMs = math.huge",
        "onError = 'no'",
    ] {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let runtime = tls_runtime();
        let problem = runtime
            .eval::<String>(&format!(
                "local http = require '@jess/http'
                local net = require '@jess/net'
                local _, problem = net.run(function()
                  http.serve {{ address = '127.0.0.1:{port}', handler = function() return {{}} end, {option} }}
                end)
                return problem"
            ))
            .unwrap_or_else(|error| panic!("{error}"));
        let name = option.split(' ').next().unwrap();
        assert!(problem.contains(name), "{option}: {problem}");
        assert!(
            TcpListener::bind(("127.0.0.1", port)).is_ok(),
            "{option} left the port listened on"
        );
        drop(runtime);
    }
}

#[test]
fn a_streamed_response_goes_out_chunk_by_chunk_as_it_is_made() {
    let server = Served::start();
    let mut stream = server.connect();
    stream
        .write_all(b"GET /stream HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
        .unwrap();
    let started = Instant::now();
    let mut received = Vec::new();
    let mut first = None;
    let mut piece = [0u8; 256];
    loop {
        let count = stream.read(&mut piece).unwrap();
        if count == 0 {
            break;
        }
        received.extend_from_slice(&piece[..count]);
        if first.is_none() && String::from_utf8_lossy(&received).contains("one ") {
            first = Some(started.elapsed());
        }
    }
    let text = String::from_utf8_lossy(&received).into_owned();
    assert!(text.contains("transfer-encoding: chunked\r\n"), "{text}");
    assert!(text.ends_with("4\r\none \r\n4\r\ntwo \r\n5\r\nthree\r\n0\r\n\r\n"), "{text}");
    assert!(
        first.unwrap() < Duration::from_millis(150),
        "the first chunk waited for the rest"
    );
    assert!(started.elapsed() >= Duration::from_millis(290));

    // An HTTP/1.0 client doesn't read chunks: its stream is the bytes, ended by the close.
    let old = exchange(&server, "GET /stream HTTP/1.0\r\n\r\n");
    assert!(!old.contains("transfer-encoding"), "{old}");
    assert!(old.contains("connection: close\r\n"), "{old}");
    assert!(old.ends_with("\r\n\r\none two three"), "{old}");
    drop(server.finish());
}

#[test]
fn slow_handlers_on_separate_connections_run_at_once_and_connections_are_bounded() {
    let server = Served::start();
    let started = Instant::now();
    let slow: Vec<_> = (0..3)
        .map(|_| {
            let mut stream = server.connect();
            stream
                .write_all(b"GET /slow HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
                .unwrap();
            stream
        })
        .collect();
    for mut stream in slow {
        assert!(read_all(&mut stream).ends_with("slow"));
    }
    // Three 300 ms handlers one after another would take 900 ms; together they take about 300.
    assert!(started.elapsed() < Duration::from_millis(850), "{:?}", started.elapsed());

    // Four connections that send nothing hold every place; a fifth is not answered until one of them
    // goes, well before their head timeout would free one.
    let mut idle: Vec<TcpStream> = (0..4).map(|_| server.connect()).collect();
    thread::sleep(Duration::from_millis(50));
    let mut fifth = server.connect();
    fifth
        .write_all(b"GET /hello HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
        .unwrap();
    fifth.set_read_timeout(Some(Duration::from_millis(150))).unwrap();
    let mut early = [0u8; 1];
    assert!(
        fifth.read(&mut early).is_err(),
        "the fifth was answered while every place was held"
    );
    drop(idle.remove(0).shutdown(Shutdown::Both));
    fifth.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert!(
        read_all(&mut fifth).ends_with("hi"),
        "a place came free and the fifth was served"
    );
    for stream in idle {
        drop(stream.shutdown(Shutdown::Both));
    }
    drop(server.finish());
}

#[test]
fn jess_s_client_reuses_a_connection_to_jess_s_server_over_tcp_and_tls() {
    let lines = run_task(
        &tls_runtime(),
        "
        local tls = require '@dream/tls'
        local sse = require '@jess/sse'
        local function handle(request: http.ServerRequest): http.ServerResponse
          if request.path == '/events' then
            return {
              headers = { ['Content-Type'] = 'text/event-stream' },
              stream = function(send)
                for index = 1, 3 do
                  send(`id: {index}\\ndata: event {index}\\n\\n`)
                  net.sleep(20)
                end
              end,
            }
          end

          return { body = `{request.method} {request.path} {request.secure}` }
        end

        local plain = assert(http.serve { address = '127.0.0.1:0', handler = handle })
        local client = http.client()
        for _ = 1, 5 do
          local response = assert(client:request { url = `http://127.0.0.1:{plain.port}/again` })
          assert(buffer.tostring(response.body) == 'GET /again false')
        end
        local stats = plain:stats()
        table.insert(out, `plain connections {stats.connections} requests {stats.requests}`)

        local events = assert(sse.open { url = `http://127.0.0.1:{plain.port}/events`, client = client })
        local seen = {}
        while true do
          local event = events:next(net.now() + 5000)
          if not event then break end
          table.insert(seen, `{event.id}:{event.data}`)
        end
        table.insert(out, table.concat(seen, ' '))
        client:close()

        local config = assert(tls.serverConfig { certChain = SERVER_CERT, privateKey = SERVER_KEY })
        local secure = assert(http.serve { address = '127.0.0.1:0', handler = handle, tls = config })
        local trusted = assert(tls.clientConfig { roots = CA, platform = false })
        local response = assert(http.request { url = `https://localhost:{secure.port}/secret`, tls = trusted })
        table.insert(out, buffer.tostring(response.body))

        plain:close()
        secure:close()
        table.insert(out, `closed {plain.closed} {secure:stats().connections}`)
        ",
    );
    assert_eq!(
        lines,
        [
            "plain connections 1 requests 5",
            "1:event 1 2:event 2 3:event 3",
            "GET /secret true",
            "closed true 0",
        ]
    );
}

#[test]
fn nothing_listens_without_the_runtime_s_grant() {
    let denied = jess::lua::runtime_with_capabilities(Options::default(), &[]).unwrap();
    let error = denied
        .eval::<String>(
            "local http = require '@jess/http'
            local net = require '@jess/net'
            local _, problem = net.run(function()
              http.serve { address = '127.0.0.1:0', handler = function() return {} end }
            end)
            return problem",
        )
        .unwrap_or_else(|error| error.to_string());
    assert!(error.contains("network.tcp.listen"), "{error}");
}

#[test]
fn net_release_wakes_a_task_waiting_on_a_listener_so_it_can_be_closed_at_once() {
    let lines = run_task(
        &tls_runtime(),
        "
        local tcp = require '@dream/tcp'
        local listener = assert(tcp.listen('127.0.0.1:0'))
        local woke = ''
        local waiter = net.spawn(function()
          local ready, reason = net.wait(listener, 'read')
          woke = `{ready} {reason}`
        end)
        net.sleep(20)
        net.release(listener)
        listener:close()
        -- Releasing a handle nobody waits on does nothing.
        net.release(listener)
        table.insert(out, `joined {waiter:join(net.now() + 1000)}`)
        table.insert(out, woke)
        ",
    );
    assert_eq!(lines, ["joined true", "false closed"]);
}
