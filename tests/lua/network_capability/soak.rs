//! The modules together, the way a long-running application uses them: a WebSocket heartbeat
//! that keeps its time while an event stream trickles in and requests go through a pooled client,
//! and repeated cycles of requests, cancelled streams and sockets that leave the loop, the pool
//! and the heap where they started.

use std::time::Duration;

use jess::lua::l3i::Runtime;
use jess::lua::l3i::memory::GcControl;

use super::http::{Answer, Server};
use super::websocket::WsServer;
use super::{network_runtime, run_task};

fn run(runtime: &Runtime, body: &str) -> Vec<String> {
    run_task(
        runtime,
        &format!("local sse = require '@jess/sse'\nlocal websocket = require '@jess/websocket'\n{body}"),
    )
}

fn kept(body: &str) -> String {
    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len())
}

/// The longest silence between consecutive messages the WebSocket server received.
fn longest_gap(server: &WsServer) -> Duration {
    let arrivals: Vec<_> = server.seen.lock().unwrap().arrivals.iter().map(|(at, _)| *at).collect();
    arrivals.windows(2).map(|pair| pair[1] - pair[0]).max().unwrap_or_default()
}

#[test]
fn a_heartbeat_keeps_time_beside_an_event_stream_and_pooled_requests() {
    let gateway = WsServer::start();
    let mut pieces = vec![(
        Duration::ZERO,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n".to_owned(),
    )];
    pieces.extend((1..=12).map(|index| (Duration::from_millis(150), format!("event: token\ndata: {index}\n\n"))));
    let api = Server::start(vec![
        ("/generate", Answer::Trickle(pieces)),
        ("/issue", Answer::Kept(kept("{}"))),
    ]);
    let lines = run(
        &network_runtime(),
        &format!(
            "
            local socket = assert(websocket.connect({gateway:?}))
            local beating = true
            local beats = loop:spawn(function()
              local count = 0
              while beating do
                net.sleep(50)
                count += 1
                assert(socket:send(`beat {{count}}`))
              end
            end, 'heartbeat')

            local client = http.client {{ maxConnectionsPerOrigin = 2 }}
            local requests = loop:spawn(function()
              for _ = 1, 20 do
                assert(client:request {{ url = {issue:?} }})
                net.sleep(20)
              end
            end, 'requests')

            local events = assert(sse.open {{ url = {generate:?}, client = client }})
            local tokens = {{}}
            while true do
              local event, problem = events:next()
              if not event then assert(not problem, problem) break end
              table.insert(tokens, event.data)
            end

            assert(requests:join(net.now() + 10000))
            beating = false
            assert(beats:join(net.now() + 1000))
            socket:close()
            client:close()
            table.insert(out, table.concat(tokens, ' '))
            table.insert(out, `idle {{client:stats().idle}} busy {{client:stats().busy}}`)
            ",
            gateway = gateway.url("/sink"),
            issue = api.url("/issue"),
            generate = api.url("/generate"),
        ),
    );
    assert_eq!(lines, ["1 2 3 4 5 6 7 8 9 10 11 12", "idle 0 busy 0"]);
    let gap = longest_gap(&gateway);
    assert!(gap < Duration::from_millis(300), "the heartbeat stalled for {gap:?}");
    assert!(
        gateway.seen.lock().unwrap().arrivals.len() >= 25,
        "the heartbeat ran the whole stream"
    );
    assert!(
        api.accepted() <= 3,
        "{} connections: two pooled, one for the stream",
        api.accepted()
    );
    assert_eq!(gateway.wait_until_idle(), 0);
}

#[test]
fn repeated_requests_cancelled_streams_and_sockets_leave_nothing_behind() {
    let gateway = WsServer::start();
    let stall = vec![
        (
            Duration::ZERO,
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npart".to_owned(),
        ),
        (Duration::from_millis(1500), String::new()),
    ];
    let api = Server::start(vec![("/kept", Answer::Kept(kept("ok"))), ("/stall", Answer::Trickle(stall))]);
    let runtime = network_runtime();
    let cycles = |count: usize| {
        run(
            &runtime,
            &format!(
                "
                local client = http.client {{ maxConnectionsPerOrigin = 2 }}
                for _ = 1, {count} do
                  assert(client:request {{ url = {kept:?} }})

                  local stream = assert(client:open {{ url = {stall:?} }})
                  local reader = loop:spawn(function()
                    assert(stream:read() == 'part')
                    local piece, _, kind = stream:read()
                    assert(piece == nil and kind == 'cancelled', kind)
                  end)
                  net.sleep(10)
                  reader:cancel()
                  reader:join(net.now() + 1000)

                  local socket = assert(websocket.connect({gateway:?}))
                  assert(socket:send('cycle'))
                  socket:close()
                end

                local pooled = client:stats()
                client:close()
                local stats = loop:stats()
                table.insert(out, `pooled busy {{pooled.busy}} idle at most {{pooled.idle <= 2}}`)
                table.insert(out, `tasks {{stats.tasks}} connections {{stats.connections}} watching {{stats.watching}}`)
                ",
                kept = api.url("/kept"),
                stall = api.url("/stall"),
                gateway = gateway.url("/sink"),
            ),
        )
    };
    let heap = || {
        runtime.collect_garbage();
        runtime.gc(GcControl::Count)
    };

    let settled = ["pooled busy 0 idle at most true", "tasks 1 connections 0 watching 0"];
    assert_eq!(cycles(5), settled);
    let before = heap();
    assert_eq!(cycles(60), settled, "nothing is left on the loop or in the pool");
    let after = heap();
    assert!(
        after - before < 512,
        "the heap grew from {before} KiB to {after} KiB over 60 cycles"
    );
    assert_eq!(gateway.wait_until_idle(), 0, "every socket closed");
    assert_eq!(api.requests(), 2 * 65, "a request and a stream per cycle");
}
