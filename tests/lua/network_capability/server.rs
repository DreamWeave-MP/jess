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
local headTimeout, reportPort = ...
local errors: { string } = {}
local together, steps = 0, 0
local allTogether = net.signal()

--- Waits until `/step` has been asked for `count` times, so a test can read each chunk of `/stepped`
--- before the next is sent. The limit stops a hang.
local function awaitStep(count: number)
  local limit = net.now() + 30000
  while steps < count and net.now() < limit do
    net.sleep(5)
  end

  assert(steps >= count, `timed out waiting for step {count}`)
end

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

  -- Each of three handlers waits for the others: handlers run one at a time would wait out the
  -- limit and answer alone.
  if request.path == '/together' then
    together += 1
    if together == 3 then
      allTogether:notify()
    elseif not allTogether:wait(net.now() + 30000) then
      return { body = 'alone' }
    end

    return { body = 'together' }
  end

  if request.path == '/step' then
    steps += 1
    return { body = 'stepped' }
  end

  -- Holds its connection's place until the client goes, and says so with its first chunk.
  if request.path == '/hold' then
    return {
      stream = function(send: (data: string | buffer) -> boolean)
        local limit = net.now() + 60000
        while send('.') and net.now() < limit do
          net.sleep(20)
        end
      end,
    }
  end

  if request.path == '/stepped' then
    return {
      headers = { ['Content-Type'] = 'text/plain' },
      stream = function(send: (data: string | buffer) -> boolean)
        assert(send('one '))
        awaitStep(1)
        assert(send('two '))
        awaitStep(2)
        assert(send('three'))
      end,
    }
  end

  if request.path == '/stream' then
    return {
      headers = { ['Content-Type'] = 'text/plain' },
      stream = function(send: (data: string | buffer) -> boolean)
        send('one ')
        send('two ')
        send('three')
      end,
    }
  end

  return { status = 404, body = 'missing' }
end

local quit = net.signal()
local finished, problem = net.run(function()
  local server = assert(http.serve {
    address = '127.0.0.1:0',
    maxConnections = 4,
    maxHeadBytes = 2048,
    maxBodyBytes = 1000,
    headTimeoutMs = headTimeout,
    handler = function(request: http.ServerRequest): http.ServerResponse
      if request.path == '/quit' then
        quit:notify()
        return { body = 'bye' }
      end

      return handle(request)
    end,
    onError = function(message: string) table.insert(errors, message) end,
  })

  -- Tells the test the port it got. A port chosen beforehand and bound here could be taken in
  -- between by a server another test starts.
  local report = assert(net.connect('127.0.0.1', reportPort))
  assert(report:send(tostring(server.port)))
  report:close()

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
    /// A server whose head timeout no slow machine reaches between a connection and its request.
    fn start() -> Self {
        Self::with_head_timeout(60_000)
    }

    fn with_head_timeout(milliseconds: u32) -> Self {
        let report = TcpListener::bind("127.0.0.1:0").unwrap();
        let report_port = report.local_addr().unwrap().port();
        report.set_nonblocking(true).unwrap();
        let done = thread::spawn(move || {
            let capabilities = ["network.tcp.listen", "network.tcp.connect", "network.dns.resolve"].map(str::to_owned);
            let runtime = jess::lua::runtime_with_capabilities(Options::default(), &capabilities).unwrap();
            let script = format!("return (function(...) {SERVER} end)({milliseconds}, {report_port})");
            runtime.eval::<String>(&script).unwrap_or_else(|error| panic!("{error}"))
        });
        let started = Instant::now();
        let mut told = loop {
            match report.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(!done.is_finished(), "the server ended before it listened");
                    assert!(started.elapsed() < Duration::from_secs(60), "the server never listened");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        told.set_nonblocking(false).unwrap();
        told.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let port = read_all(&mut told).parse().unwrap();
        Self { port, done: Some(done) }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
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

/// Reads into `received` until it holds `text`; the stream's read timeout fails the test.
fn read_until(stream: &mut TcpStream, received: &mut Vec<u8>, text: &str) {
    let mut piece = [0u8; 256];
    while !String::from_utf8_lossy(received).contains(text) {
        let count = stream.read(&mut piece).unwrap();
        assert_ne!(count, 0, "the stream ended before {text:?}");
        received.extend_from_slice(&piece[..count]);
    }
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
    let server = Served::with_head_timeout(400);
    // Before the connection, since the server's timeout starts from it.
    let started = Instant::now();
    let mut stream = server.connect();
    stream.write_all(b"GET /hello HTTP/1.1\r\nHost: t\r\n").unwrap();
    let answer = read_all(&mut stream);
    assert_eq!(answer, "", "a half-sent head gets nothing");
    let waited = started.elapsed();
    // The upper bound tells the head timeout from the read timeout this side gave up at.
    assert!(
        waited >= Duration::from_millis(390) && waited < Duration::from_secs(25),
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
              local response = assert(http.request { url = `http://127.0.0.1:{server.port}/fine`, timeoutMs = 30000 })
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
        local function serve(headTimeoutMs: number)
          return assert(http.serve {
            address = '127.0.0.1:0',
            tls = config,
            maxConnections = 2,
            headTimeoutMs = headTimeoutMs,
            handler = function() return { body = 'fine' } end,
          })
        end

        -- The timeout only one handshake below is meant to reach is on a server of its own, so a
        -- slow machine's handshakes on the other never run into it.
        local server, hasty = serve(120000), serve(300)

        -- More refused handshakes than the server has places.
        for _ = 1, 6 do
          local raw = assert(net.connect('127.0.0.1', server.port))
          assert(raw:send('this is not a TLS client hello\\r\\n\\r\\n'))
          while raw:receiveSome(4096, net.now() + 30000) do end
          raw:close()
        end

        -- And one that says nothing until its handshake times out.
        local silent = assert(net.connect('127.0.0.1', hasty.port))
        while silent:receiveSome(4096, net.now() + 30000) do end
        silent:close()
        table.insert(out, `after failures {server:stats().connections} {hasty:stats().connections}`)
        hasty:close()

        local trusted = assert(tls.clientConfig { roots = CA, platform = false })
        local response = assert(http.request { url = `https://localhost:{server.port}/`, tls = trusted, timeoutMs = 30000 })
        table.insert(out, buffer.tostring(response.body))

        -- A handshake still waiting when the server closes is closed with it.
        local pending = assert(net.connect('127.0.0.1', server.port))
        while server:stats().connections == 0 do
          net.sleep(5)
        end

        table.insert(out, `pending {server:stats().connections}`)
        server:close()
        table.insert(out, `closed {server:stats().connections}`)
        table.insert(out, `pending ended {pending:receiveSome(10, net.now() + 30000) == nil}`)
        pending:close()
        ",
    );
    assert_eq!(
        lines,
        ["after failures 0 0", "fine", "pending 1", "closed 0", "pending ended true"]
    );
}

/// The acceptor counts a connection the moment it accepts it, and the connection's task runs later:
/// a server closed in between closes the socket under a task that hasn't started, which then
/// finds it closed instead of raising on it.
#[test]
fn a_server_closed_between_accepting_a_connection_and_serving_it_closes_it_quietly() {
    let lines = run_task(
        &tls_runtime(),
        "
        local tcp = require '@dream/tcp'
        local tls = require '@dream/tls'
        local config = assert(tls.serverConfig { certChain = SERVER_CERT, privateKey = SERVER_KEY })
        for _, secure in { false, true } do
          local server = assert(http.serve {
            address = '127.0.0.1:0',
            tls = if secure then config else nil,
            handler = function() return { body = 'fine' } end,
          })
          -- The acceptor starts waiting on its listener.
          net.yield()
          local raw = assert(tcp.connect(`127.0.0.1:{server.port}`))
          -- A yielded task wakes at the end of the next step, in the same batch as the tasks that
          -- step's poll woke and after them. So the step that wakes the acceptor wakes this right
          -- after it accepts, before the connection's task, which runs in the round after. That
          -- holds however long the platform takes to report the connection.
          local limit = net.now() + 30000
          repeat
            net.yield()
            assert(net.now() < limit, 'the connection was never accepted')
          until server:stats().connections > 0

          table.insert(out, `{secure} accepted {server:stats().connections}`)
          server:close()
          net.sleep(20)
          raw:close()
        end
        ",
    );
    assert_eq!(lines, ["false accepted 1", "true accepted 1"]);
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
        .write_all(b"GET /stepped HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
        .unwrap();
    // Each chunk goes out only once the one before it has arrived here, so a server that held them
    // back until the end never gets as far as sending this test what it reads for.
    let step = "GET /step HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n";
    let mut received = Vec::new();
    read_until(&mut stream, &mut received, "one ");
    assert!(exchange(&server, step).ends_with("stepped"));
    read_until(&mut stream, &mut received, "two ");
    assert!(exchange(&server, step).ends_with("stepped"));
    drop(stream.read_to_end(&mut received));
    let text = String::from_utf8_lossy(&received).into_owned();
    assert!(text.contains("transfer-encoding: chunked\r\n"), "{text}");
    assert!(text.ends_with("4\r\none \r\n4\r\ntwo \r\n5\r\nthree\r\n0\r\n\r\n"), "{text}");

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
    let slow: Vec<_> = (0..3)
        .map(|_| {
            let mut stream = server.connect();
            stream
                .write_all(b"GET /together HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
                .unwrap();
            stream
        })
        .collect();
    for mut stream in slow {
        let answer = read_all(&mut stream);
        assert!(answer.ends_with("together"), "{answer}");
    }

    // Four connections held by their handlers hold every place, each known to by its first chunk; a
    // fifth is not answered until one of them goes.
    let mut held: Vec<TcpStream> = (0..4)
        .map(|_| {
            let mut stream = server.connect();
            stream.write_all(b"GET /hold HTTP/1.1\r\nHost: t\r\n\r\n").unwrap();
            read_until(&mut stream, &mut Vec::new(), "\r\n\r\n1\r\n.");
            stream
        })
        .collect();
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
    drop(held.remove(0).shutdown(Shutdown::Both));
    fifth.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    assert!(
        read_all(&mut fifth).ends_with("hi"),
        "a place came free and the fifth was served"
    );
    for stream in held {
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
          local event = events:next(net.now() + 30000)
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
        table.insert(out, `joined {waiter:join(net.now() + 30000)}`)
        table.insert(out, woke)
        ",
    );
    assert_eq!(lines, ["joined true", "false closed"]);
}

#[test]
fn a_server_whose_acceptor_cannot_start_closes_its_listener() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let runtime = tls_runtime();
    let failure = runtime
        .eval::<String>(&format!(
            "local http = require '@jess/http'
            local net = require '@jess/net'
            local failures: {{ string }} = {{}}
            local loop = net.loop {{ maxTasks = 1, onError = function(_task, message) table.insert(failures, message) end }}
            loop:spawn(function()
              http.serve {{ address = '127.0.0.1:{port}', handler = function() return {{}} end }}
            end)
            loop:run(2000)
            loop:close()
            return failures[1] or ''"
        ))
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(failure.contains("the most it allows"), "{failure}");
    assert!(
        TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "the listener outlived the server that couldn't start"
    );
}

#[test]
fn a_handle_is_waited_on_from_one_loop_at_a_time() {
    let text = tls_runtime()
        .eval::<String>(
            "
            local net = require '@jess/net'
            local tcp = require '@dream/tcp'
            local listener = assert(tcp.listen('127.0.0.1:0'))
            local failures: { string } = {}
            local reasons: { string } = {}
            local first = net.loop()
            local second = net.loop { onError = function(_task, message) table.insert(failures, message) end }
            first:spawn(function() net.wait(listener, 'read') end)
            first:step(0)
            second:spawn(function() net.wait(listener, 'read') end)
            second:step(0)

            -- Once the first loop lets go of it, the second may wait on it.
            first:close()
            second:spawn(function()
              local _, reason = net.wait(listener, 'read', net.now() + 10)
              table.insert(reasons, reason)
            end)
            second:run(1000)
            second:close()
            listener:close()
            return `{failures[1]}\\n{reasons[1]}`
            ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
    let lines: Vec<_> = text.lines().collect();
    assert!(lines[0].contains("another loop is already waiting on this handle"), "{text}");
    assert_eq!(lines[1], "timeout", "{text}");
}

/// Two tasks woken by one poll: whichever runs first closes the other's connection, whose task has
/// been told it is ready and hasn't run yet. That task hears its connection closed; it doesn't go on
/// to read a closed stream, which raises.
#[test]
fn a_connection_closed_under_a_task_already_woken_ends_its_receive_as_closed() {
    let lines = run_task(
        &tls_runtime(),
        "
        local tcp = require '@dream/tcp'
        local listener = assert(tcp.listen('127.0.0.1:0'))
        local port = assert(tonumber(string.match(listener.localAddress, ':(%d+)$')))
        local connections = { assert(net.connect('127.0.0.1', port)), assert(net.connect('127.0.0.1', port)) }
        local peers = {}
        while #peers < 2 do
          local peer, _, kind = listener:accept()
          if peer then
            table.insert(peers, peer)
            continue
          end

          assert(kind == 'wouldBlock', kind)
          assert(net.wait(listener, 'read', net.now() + 30000))
        end

        local ended = {}
        local tasks = {}
        for index, connection in connections do
          table.insert(tasks, loop:spawn(function()
            local piece, _, kind = connection:receiveSome(16)
            table.insert(ended, piece or kind)
            if piece then connections[3 - index]:close() end
          end))
        end

        -- Both tasks park on this yield; both peers then write before the next poll.
        net.yield()
        for _, peer in peers do
          assert(peer:write('x', 0, 1))
        end

        for _, task in tasks do
          assert(task:join(net.now() + 30000))
        end

        table.sort(ended)
        table.insert(out, table.concat(ended, ' '))
        for _, peer in peers do
          peer:close()
        end

        listener:close()
        ",
    );
    assert_eq!(lines, ["closed x"]);
}
