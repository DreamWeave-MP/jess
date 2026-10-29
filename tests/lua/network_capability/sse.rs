//! `@jess/sse` on the wire: an event stream written to the client in pieces that split lines,
//! line endings and events wherever they fall, against the parsing the WHATWG standard sets out;
//! limits; and what isn't an event stream.

use std::time::Duration;

use super::http::{Answer, Server, response};
use super::{network_runtime, run_task};

const HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";

fn stream(pieces: &[&str]) -> Answer {
    let pause = Duration::from_millis(60);
    let mut written = vec![(Duration::ZERO, HEAD.to_owned())];
    written.extend(pieces.iter().map(|piece| (pause, (*piece).to_owned())));
    Answer::Trickle(written)
}

fn run(body: &str) -> Vec<String> {
    run_task(&network_runtime(), &format!("local sse = require '@jess/sse'\n{body}"))
}

#[test]
fn events_are_parsed_as_the_standard_says_wherever_the_pieces_split_them() {
    let server = Server::start(vec![(
        "/events",
        stream(&[
            "\u{feff}:a comment\n\ndata: one\n\nevent: update\ndata: two\ndata: lines\nid: 7\n\n",
            "retry: 2500\ndata: split acr",
            "oss writes\r",
            "\n\r\ndata:nospace\rid\r\r",
            "data: never dispatched",
        ]),
    )]);
    let lines = run(&format!(
        "
        local events = assert(sse.open {{ url = {url:?}, lastEventId = '3' }})
        while true do
          local event, problem = events:next(net.now() + 5000)
          if not event then
            table.insert(out, `end {{problem}} ended {{events.ended}}`)
            break
          end
          table.insert(out, `{{event.event}}|{{string.gsub(event.data, '\\n', '/')}}|{{event.id}}`)
        end
        table.insert(out, `retry {{events.retryMs}} last {{events.lastEventId}}`)
        ",
        url = server.url("/events")
    ));
    assert_eq!(
        lines,
        [
            "message|one|3",
            "update|two/lines|7",
            "message|split across writes|7",
            "message|nospace|",
            "end nil ended true",
            "retry 2500 last ",
        ]
    );
    let heads = server.heads();
    assert!(heads[0].contains("accept: text/event-stream\r\n"), "{}", heads[0]);
    assert!(heads[0].contains("last-event-id: 3\r\n"), "{}", heads[0]);
}

#[test]
fn an_event_past_its_limit_and_a_response_that_is_not_a_stream_are_refused() {
    let server = Server::start(vec![
        ("/huge", stream(&[&format!("data: {}\n\n", "x".repeat(2000))])),
        ("/endless-line", stream(&[&"y".repeat(3000)])),
        ("/missing", Answer::Raw(response(404, "no such stream", &[]))),
        (
            "/json",
            Answer::Raw(response(200, "{}", &[("Content-Type", "application/json")])),
        ),
    ]);
    let lines = run(&format!(
        "
        for _, path in {{ '/huge', '/endless-line' }} do
          local events = assert(sse.open {{ url = {base:?} .. path, maxEventBytes = 1000 }})
          local event, problem, kind = events:next(net.now() + 5000)
          table.insert(out, `{{path}} {{event == nil}} {{kind}} ended {{events.ended}}`)
        end

        for _, path in {{ '/missing', '/json' }} do
          local events, problem, kind = sse.open {{ url = {base:?} .. path }}
          table.insert(out, `{{path}} {{events == nil}} {{kind}} {{string.find(problem, 'answered', 1, true) ~= nil}}`)
        end
        ",
        base = server.url("")
    ));
    assert_eq!(
        lines,
        [
            "/huge true tooLarge ended true",
            "/endless-line true tooLarge ended true",
            "/missing true notEventStream true",
            "/json true notEventStream true",
        ]
    );
}

#[test]
fn an_option_it_does_not_take_raises() {
    let error = network_runtime()
        .eval::<()>("require('@jess/sse').open { url = 'http://127.0.0.1:1/', retries = 3 }")
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown jess.sse.open option 'retries'"), "{error}");
}

#[test]
fn a_byte_order_mark_split_across_reads_is_still_dropped() {
    let pause = Duration::from_millis(60);
    let server = Server::start(vec![(
        "/bom",
        Answer::TrickleBytes(vec![
            (Duration::ZERO, HEAD.as_bytes().to_vec()),
            (pause, vec![0xEF]),
            (pause, vec![0xBB]),
            (pause, vec![0xBF]),
            (pause, b"data: after the mark\n\n".to_vec()),
        ]),
    )]);
    let lines = run(&format!(
        "
        local events = assert(sse.open {{ url = {url:?} }})
        local event = events:next(net.now() + 5000)
        table.insert(out, if event then event.data else 'no event')
        ",
        url = server.url("/bom")
    ));
    assert_eq!(lines, ["after the mark"]);
}

#[test]
fn a_refused_stream_names_its_url_without_the_query() {
    let server = Server::start(vec![("/feed", Answer::Raw(response(401, "expired", &[])))]);
    let lines = run(&format!(
        "
        local events, problem, kind = sse.open {{ url = {url:?} }}
        table.insert(out, `{{events == nil}} {{kind}}`)
        table.insert(out, problem :: string)
        ",
        url = server.url("/feed?token=SECRET")
    ));
    assert_eq!(lines[0], "true notEventStream");
    assert!(lines[1].contains("401") && lines[1].contains("/feed"), "{}", lines[1]);
    assert!(!lines[1].contains("SECRET") && !lines[1].contains("token"), "{}", lines[1]);
}
