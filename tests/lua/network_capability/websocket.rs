//! `@jess/websocket` on the wire, against a loopback RFC 6455 server written here, independently
//! of the client: its own SHA-1 for the accept key, its own frame parser. The server records what
//! the client sent (mask bits, mask keys, pongs, the close frame) so a test can hold the client to
//! the protocol, not only to its own idea of it. `wss://` runs through the Luau TLS front in
//! `fixtures/tls_front.jsl`, in front of the same server.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use jess::lua::l3i::Runtime;

use super::{network_runtime, tls_runtime};

/// What the server saw from clients, across every connection.
#[derive(Default)]
pub(super) struct Seen {
    pub(super) accepted: usize,
    pub(super) open: usize,
    pub(super) masked: usize,
    pub(super) unmasked: usize,
    pub(super) keys: Vec<[u8; 4]>,
    pub(super) pongs: Vec<Vec<u8>>,
    pub(super) closes: Vec<(u16, String)>,
    /// Arrival times of text messages, for the long-running application test.
    pub(super) arrivals: Vec<(std::time::Instant, String)>,
}

pub(super) struct WsServer {
    pub(super) port: u16,
    pub(super) seen: Arc<Mutex<Seen>>,
}

impl WsServer {
    pub(super) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let shared = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let seen = Arc::clone(&shared);
                thread::spawn(move || {
                    {
                        let mut seen = seen.lock().unwrap();
                        seen.accepted += 1;
                        seen.open += 1;
                    }
                    serve(stream, &seen);
                    seen.lock().unwrap().open -= 1;
                });
            }
        });
        Self { port, seen }
    }

    pub(super) fn url(&self, path: &str) -> String {
        format!("ws://127.0.0.1:{}{path}", self.port)
    }

    /// Waits up to two seconds for every connection to have ended on the server's side.
    pub(super) fn wait_until_idle(&self) -> usize {
        for _ in 0..200 {
            if self.seen.lock().unwrap().open == 0 {
                return 0;
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.seen.lock().unwrap().open
    }
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut words = [0u32; 80];
        for (index, word) in block.chunks(4).enumerate() {
            words[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..80 {
            words[index] = (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16]).rotate_left(1);
        }
        let [mut first, mut second, mut third, mut fourth, mut fifth] = state;
        for (index, word) in words.iter().enumerate() {
            let (mixed, constant) = match index {
                0..=19 => ((second & third) | (!second & fourth), 0x5A82_7999),
                20..=39 => (second ^ third ^ fourth, 0x6ED9_EBA1),
                40..=59 => ((second & third) | (second & fourth) | (third & fourth), 0x8F1B_BCDC),
                _ => (second ^ third ^ fourth, 0xCA62_C1D6),
            };
            let next = first
                .rotate_left(5)
                .wrapping_add(mixed)
                .wrapping_add(fifth)
                .wrapping_add(constant)
                .wrapping_add(*word);
            fifth = fourth;
            fourth = third;
            third = second.rotate_left(30);
            second = first;
            first = next;
        }
        for (slot, value) in state.iter_mut().zip([first, second, third, fourth, fifth]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut digest = [0u8; 20];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in data.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if index <= chunk.len() {
                text.push(ALPHABET[((bits >> shift) & 0x3F) as usize] as char);
            } else {
                text.push('=');
            }
        }
    }
    text
}

fn accept_key(key: &str) -> String {
    base64(&sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()))
}

/// A frame as the server sends it: unmasked unless asked, `first` the FIN, RSV and opcode byte.
fn frame(first: u8, payload: &[u8], masked: bool) -> Vec<u8> {
    let mut bytes = vec![first];
    let mask_bit = if masked { 0x80 } else { 0 };
    match payload.len() {
        length @ 0..=125 => bytes.push(mask_bit | u8::try_from(length).unwrap()),
        length @ 126..=65535 => {
            bytes.push(mask_bit | 0x7E);
            bytes.extend_from_slice(&u16::try_from(length).unwrap().to_be_bytes());
        }
        length => {
            bytes.push(mask_bit | 0x7F);
            bytes.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    if masked {
        bytes.extend_from_slice(&[1, 2, 3, 4]);
        bytes.extend(payload.iter().enumerate().map(|(index, byte)| byte ^ [1, 2, 3, 4][index % 4]));
    } else {
        bytes.extend_from_slice(payload);
    }
    bytes
}

/// One client frame: FIN and opcode, and the payload unmasked. Records whether it was masked.
fn read_frame(stream: &mut TcpStream, seen: &Mutex<Seen>) -> Option<(bool, u8, Vec<u8>)> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).ok()?;
    let mut length = u64::from(header[1] & 0x7F);
    if length == 126 {
        let mut extended = [0u8; 2];
        stream.read_exact(&mut extended).ok()?;
        length = u64::from(u16::from_be_bytes(extended));
    } else if length == 127 {
        let mut extended = [0u8; 8];
        stream.read_exact(&mut extended).ok()?;
        length = u64::from_be_bytes(extended);
    }
    let masked = header[1] & 0x80 != 0;
    let mut key = [0u8; 4];
    if masked {
        stream.read_exact(&mut key).ok()?;
    }
    let mut payload = vec![0u8; usize::try_from(length).unwrap()];
    stream.read_exact(&mut payload).ok()?;
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= key[index % 4];
    }
    {
        let mut seen = seen.lock().unwrap();
        if masked {
            seen.masked += 1;
            seen.keys.push(key);
        } else {
            seen.unmasked += 1;
        }
    }
    Some((header[0] & 0x80 != 0, header[0] & 0x0F, payload))
}

/// What the server sends first on `path`, after the handshake; false when the connection should end
/// there.
fn open(path: &str, stream: &mut TcpStream) -> bool {
    let opening: Vec<Vec<u8>> = match path {
        "/echo" => vec![frame(0x89, b"server-ping", false)],
        "/masked" => vec![frame(0x81, b"masked", true)],
        "/rsv" => vec![frame(0xC1, b"compressed?", false)],
        "/opcode" => vec![frame(0x83, b"", false)],
        "/continuation" => vec![frame(0x80, b"x", false)],
        "/bad-utf8" => vec![frame(0x81, &[0xFF, 0xFE], false)],
        "/big" => vec![frame(0x82, &[7u8; 2000], false)],
        "/close" => vec![frame(0x88, &[0x0F, 0xA1, b'b', b'y', b'e'], false)],
        "/fragments" => vec![
            frame(0x01, b"hel", false),
            frame(0x89, b"between", false),
            frame(0x00, b"lo ", false),
            frame(0x80, b"world", false),
        ],
        "/split" => {
            let whole = frame(0x81, b"helloworld", false);
            if stream.write_all(&whole[..7]).is_err() {
                return false;
            }
            thread::sleep(Duration::from_millis(400));
            vec![whole[7..].to_vec()]
        }
        "/drop" => {
            drop(stream.shutdown(Shutdown::Both));
            return false;
        }
        _ => Vec::new(),
    };
    opening.iter().all(|bytes| stream.write_all(bytes).is_ok())
}

fn serve(mut stream: TcpStream, seen: &Mutex<Seen>) {
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read_exact(&mut byte).is_err() {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let field = |name: &str| {
        head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_owned())
        })
    };
    let key = field("sec-websocket-key").unwrap_or_default();
    let path = path.split('?').next().unwrap_or("/").to_owned();

    if path == "/refuse" {
        drop(stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno"));
        return;
    }
    let accept = if path == "/bad-accept" {
        "AAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_owned()
    } else {
        accept_key(&key)
    };
    let protocol = match field("sec-websocket-protocol") {
        Some(offered) if offered.split(',').any(|item| item.trim() == "chat") => "Sec-WebSocket-Protocol: chat\r\n",
        _ => "",
    };
    let answer = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n{protocol}\r\n"
    );
    if stream.write_all(answer.as_bytes()).is_err() {
        return;
    }

    if !open(&path, &mut stream) {
        return;
    }

    while let Some((_, opcode, payload)) = read_frame(&mut stream, seen) {
        match opcode {
            0x1 | 0x2 => {
                if opcode == 0x1 {
                    let text = String::from_utf8_lossy(&payload).into_owned();
                    seen.lock().unwrap().arrivals.push((std::time::Instant::now(), text));
                }
                if path != "/sink" && stream.write_all(&frame(0x80 | opcode, &payload, false)).is_err() {
                    return;
                }
            }
            0x9 => {
                if stream.write_all(&frame(0x8A, &payload, false)).is_err() {
                    return;
                }
            }
            0xA => seen.lock().unwrap().pongs.push(payload),
            0x8 => {
                let code = if payload.len() >= 2 {
                    u16::from_be_bytes([payload[0], payload[1]])
                } else {
                    1005
                };
                let reason = String::from_utf8_lossy(payload.get(2..).unwrap_or_default()).into_owned();
                seen.lock().unwrap().closes.push((code, reason));
                if path != "/close" {
                    drop(stream.write_all(&frame(0x88, &payload, false)));
                }
                drop(stream.shutdown(Shutdown::Both));
                return;
            }
            _ => return,
        }
    }
}

/// `body` run as a task by [`run_task`], with `@jess/websocket` in scope as `websocket`.
fn run_task(runtime: &Runtime, body: &str) -> Vec<String> {
    super::run_task(runtime, &format!("local websocket = require '@jess/websocket'\n{body}"))
}

#[test]
fn messages_round_trip_masked_under_fresh_keys_and_the_close_handshake_completes() {
    let server = WsServer::start();
    let lines = run_task(
        &network_runtime(),
        &format!(
            "
            local socket = assert(websocket.connect({url:?}, {{ protocols = {{ 'chat', 'other' }} }}))
            table.insert(out, `protocol {{socket.protocol}} {{socket.state}}`)
            assert(socket:send('hello'))
            local message = assert(socket:receive(net.now() + 5000))
            table.insert(out, `{{message.kind}} {{message.data}}`)
            for _, size in {{ 300, 70000 }} do
              local payload = string.rep(string.char(size % 251), size)
              assert(socket:send(payload, 'binary'))
              local back = assert(socket:receive(net.now() + 5000))
              table.insert(out, `{{back.kind}} {{#back.data}} {{back.data == payload}}`)
            end
            assert(socket:ping('are you there'))
            assert(socket:send('after the ping'))
            local after = assert(socket:receive(net.now() + 5000))
            table.insert(out, `{{after.data}} pong {{socket.lastPongAt ~= nil}}`)
            socket:close(1000, 'done')
            table.insert(out, `{{socket.state}} {{socket.closeCode}} {{socket.closeReason}}`)
            local sent, _, kind = socket:send('too late')
            table.insert(out, `late {{sent}} {{kind}}`)
            ",
            url = server.url("/echo")
        ),
    );
    assert_eq!(
        lines,
        [
            "protocol chat open",
            "text hello",
            "binary 300 true",
            "binary 70000 true",
            "after the ping pong true",
            "closed 1000 done",
            "late false closed",
        ]
    );
    assert_eq!(server.wait_until_idle(), 0);
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.unmasked, 0, "every client frame is masked");
    assert!(seen.masked >= 7, "{}", seen.masked);
    let distinct: HashSet<[u8; 4]> = seen.keys.iter().copied().collect();
    assert!(distinct.len() >= seen.keys.len() - 1, "mask keys repeat: {:?}", seen.keys);
    assert_eq!(
        seen.pongs,
        [b"server-ping".to_vec()],
        "the server's ping was answered with its payload"
    );
    assert_eq!(seen.closes, [(1000, "done".to_owned())]);
}

#[test]
fn fragments_reassemble_around_a_control_frame() {
    let server = WsServer::start();
    let lines = run_task(
        &network_runtime(),
        &format!(
            "
            local socket = assert(websocket.connect({url:?}))
            local message = assert(socket:receive(net.now() + 5000))
            table.insert(out, `{{message.kind}} {{message.data}}`)
            socket:close()
            ",
            url = server.url("/fragments")
        ),
    );
    assert_eq!(lines, ["text hello world"]);
    assert_eq!(server.wait_until_idle(), 0);
    assert_eq!(server.seen.lock().unwrap().pongs, [b"between".to_vec()]);
}

#[test]
fn a_handshake_the_server_refuses_or_answers_wrongly_fails_as_handshake() {
    let server = WsServer::start();
    let lines = run_task(
        &network_runtime(),
        &format!(
            "
            for _, url in {{ {refuse:?}, {accept:?}, 'http://127.0.0.1/', 'ws://127.0.0.1/#fragment' }} do
              local socket, message, kind = websocket.connect(url)
              table.insert(out, `{{socket == nil}} {{kind}} {{message}}`)
            end
            ",
            refuse = server.url("/refuse"),
            accept = server.url("/bad-accept"),
        ),
    );
    assert!(
        lines[0].starts_with("true handshake ") && lines[0].contains("403"),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with("true handshake ") && lines[1].contains("Sec-WebSocket-Accept"),
        "{lines:?}"
    );
    assert!(lines[2].starts_with("true unsupportedScheme"), "{lines:?}");
    assert!(lines[3].starts_with("true invalidUrl"), "{lines:?}");
    assert_eq!(server.wait_until_idle(), 0);
}

#[test]
fn malformed_frames_close_the_connection_with_the_code_that_names_them() {
    let server = WsServer::start();
    let cases = [
        ("/masked", "protocol 1002"),
        ("/rsv", "protocol 1002"),
        ("/opcode", "protocol 1002"),
        ("/continuation", "protocol 1002"),
        ("/bad-utf8", "protocol 1007"),
        ("/big", "tooLarge 1009"),
    ];
    for (path, expected) in cases {
        let lines = run_task(
            &network_runtime(),
            &format!(
                "
                local socket = assert(websocket.connect({url:?}, {{ maxMessageBytes = 1000 }}))
                local message, problem, kind = socket:receive(net.now() + 5000)
                table.insert(out, `{{kind}} {{socket.closeCode}} {{socket.state}}`)
                ",
                url = server.url(path)
            ),
        );
        assert_eq!(lines, [format!("{expected} closed")], "{path}");
    }
    assert_eq!(server.wait_until_idle(), 0);
    let closes: Vec<u16> = server.seen.lock().unwrap().closes.iter().map(|(code, _)| *code).collect();
    assert_eq!(closes, [1002, 1002, 1002, 1002, 1007, 1009], "the client told the server why");
}

#[test]
fn the_peer_s_close_and_a_dropped_connection_are_reported_with_their_codes() {
    let server = WsServer::start();
    let lines = run_task(
        &network_runtime(),
        &format!(
            "
            for _, url in {{ {close:?}, {drop:?} }} do
              local socket = assert(websocket.connect(url))
              local message, problem, kind = socket:receive(net.now() + 5000)
              table.insert(out, `{{kind}} {{socket.closeCode}} {{socket.closeReason}} {{socket.state}}`)
            end
            ",
            close = server.url("/close"),
            drop = server.url("/drop"),
        ),
    );
    assert_eq!(lines, ["closed 4001 bye closed", "closed 1006 nil closed"]);
    assert_eq!(server.wait_until_idle(), 0);
    assert_eq!(
        server.seen.lock().unwrap().closes,
        [(4001, String::new())],
        "the close is echoed"
    );
}

#[test]
fn repeated_reconnects_release_every_socket_watch_and_task() {
    use jess::lua::l3i::memory::GcControl;

    let server = WsServer::start();
    let runtime = network_runtime();
    let cycles = |count: usize| {
        run_task(
            &runtime,
            &format!(
                "
                for _ = 1, {count} do
                  local socket = assert(websocket.connect({url:?}))
                  assert(socket:send('round'))
                  assert(socket:receive(net.now() + 5000))
                  socket:close()
                end

                local stats = loop:stats()
                table.insert(out, `tasks {{stats.tasks}} connections {{stats.connections}} watching {{stats.watching}}`)
                ",
                url = server.url("/plain")
            ),
        )
    };
    let heap = || {
        runtime.collect_garbage();
        runtime.gc(GcControl::Count)
    };

    assert_eq!(cycles(10), ["tasks 1 connections 0 watching 0"]);
    let before = heap();
    assert_eq!(
        cycles(200),
        ["tasks 1 connections 0 watching 0"],
        "nothing is left on the loop"
    );
    let after = heap();
    assert!(
        after - before < 512,
        "the heap grew from {before} KiB to {after} KiB over 200 reconnects"
    );
    assert_eq!(server.wait_until_idle(), 0, "every connection was closed");
    assert_eq!(server.seen.lock().unwrap().accepted, 210);
}

#[test]
fn wss_verifies_the_server_against_the_roots_it_is_given() {
    let server = WsServer::start();
    let lines = run_task(
        &tls_runtime(),
        &format!(
            "
            local port = tlsFront(loop, {port}, SERVER_CERT, SERVER_KEY)
            local tls = require '@dream/tls'
            local trusted = assert(tls.clientConfig {{ roots = CA, platform = false }})
            local socket = assert(websocket.connect(`wss://localhost:{{port}}/echo`, {{ tls = trusted }}))
            assert(socket:send('secure hello'))
            local message = assert(socket:receive(net.now() + 5000))
            table.insert(out, `{{socket.response.status}} {{message.data}}`)
            socket:close()
            table.insert(out, `{{socket.closeCode}}`)

            local refused, problem, kind = websocket.connect(`wss://localhost:{{port}}/echo`)
            table.insert(out, `untrusted {{refused == nil}} {{kind}}`)
            ",
            port = server.port
        ),
    );
    assert_eq!(lines, ["101 secure hello", "1000", "untrusted true tls"]);
    assert_eq!(server.wait_until_idle(), 0);
}

#[test]
fn nothing_connects_without_the_runtime_s_grant() {
    let server = WsServer::start();
    let denied = super::runtime(&[]);
    let script = format!(
        "
        local net = require '@jess/net'
        local websocket = require '@jess/websocket'
        local problem
        local finished, failure = net.run(function()
          local _, message = websocket.connect({url:?})
          problem = message
        end)
        return tostring(problem or failure)
        ",
        url = server.url("/echo")
    );
    let message = denied.eval::<String>(&script).unwrap_or_else(|error| error.to_string());
    assert!(message.contains("network."), "{message}");
    assert_eq!(server.seen.lock().unwrap().accepted, 0);
}

#[test]
fn a_receive_that_times_out_part_way_through_a_frame_leaves_it_whole_for_the_next() {
    let server = WsServer::start();
    let lines = run_task(
        &network_runtime(),
        &format!(
            "
            local socket = assert(websocket.connect({url:?}))
            local early, _, kind = socket:receive(net.now() + 100)
            table.insert(out, `{{early == nil}} {{kind}} {{socket.state}}`)
            local message = assert(socket:receive(net.now() + 5000))
            table.insert(out, message.data)
            socket:close()
            ",
            url = server.url("/split")
        ),
    );
    assert_eq!(lines, ["true timeout open", "helloworld"]);
    assert_eq!(server.wait_until_idle(), 0);
}
