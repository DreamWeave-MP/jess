---
title: '@jess/sse'
description: 'Server-Sent Events over an HTTP stream: events parsed as the standard sets out, bounded in size, with the last event id kept for the caller to resume from.'
weight: 161
extra:
  kind: api
---

{{ api_signature(value="require('@jess/sse'): { open }") }}

Server-Sent Events, the `text/event-stream` responses that services stream updates and model
output as, read with [`@jess/http`'s `open`](@/docs/api/http.md#open) and parsed as the WHATWG
HTML standard says: lines may end in CRLF, LF or CR, a stream may begin with a byte-order mark,
`data` lines join with newlines, `event` names an event, `id` sets the last event id (unless it
holds a NUL), `retry` sets the reconnection delay (digits only), a line starting with `:` is a
comment, and a blank line dispatches what came before it. Pieces of the response split lines,
line endings and events wherever the network splits them; the parser waits for the rest.

It reads one stream and stops when it ends. Reconnecting is yours: what a dropped stream means
(retry at once, back off, give up) depends on the service, so the stream says what you need to
decide it, `lastEventId` and `retryMs`, and does nothing on its own.

## open

{{ api_signature(value="sse.open(options: Options) -> (EventStream?, string?, ErrorKind?)") }}

In a `@jess/net` task: opens the stream with `Accept: text/event-stream` and
`Cache-Control: no-cache`, unless `headers` names its own.

- **Parameters:** `url`, `method`, `headers`, `body`, `json`, `timeoutMs`, `readTimeoutMs`,
  `maxRedirects`, `crossOriginRedirects` and `tls` as `http.open` takes them. `lastEventId`: sent
  as `Last-Event-ID`, to resume after an event you already have. `maxEventBytes` (1 MiB): the most
  one event may take, its lines and fields together; a single line longer than that is refused
  too. `client`: an [`http.client`](@/docs/api/http.md#client) to open the stream through, so a
  stream and the requests around it share connections.
- **Returns:** the `EventStream`, or `nil`, a message and the kind: `@jess/http`'s kinds, and
  `notEventStream` for a response that isn't a `200` with `Content-Type: text/event-stream`, its
  status in the message (an expired token's `401` reads as one).

## EventStream

{{ api_signature(value="EventStream: { status, headers, url, lastEventId, retryMs, ended, next, close }") }}

{{ api_signature(value="events:next(deadline: number?) -> (Event?, string?, ErrorKind?)") }}

The next event, `{ event, data, id }`: `event` is `message` unless the server named another,
`data` its data lines joined, and `id` the last event id as of this event, empty when none has
been set. `nil` with no message when the server ended the stream (an event it never finished with
a blank line is dropped, as the standard says); `nil`, a message and the kind (`timeout`,
`closed`, `tooLarge`, `cancelled`) when it can't be read. Either way the stream has `ended`.

`lastEventId` is the id to resume from; `retryMs` is the delay the server last asked for, if it
did. `close` stops reading; a stream read to its end gives its connection back to its client.

```luau
--!strict
local net = require '@jess/net'
local sse = require '@jess/sse'

net.run(function()
  local events, message, kind = sse.open {
    url = 'https://api.example.com/v1/generate',
    method = 'POST',
    json = { prompt = 'hello', stream = true },
    readTimeoutMs = 120000,
  }

  if not events then error(`{kind}: {message}`) end

  while true do
    local event, problem = events:next()
    if not event then
      if problem then print(`stopped at {events.lastEventId}: {problem}`) end
      break
    end

    print(event.event, event.data)
  end
end)
```

Related: [@jess/http](@/docs/api/http.md), [@jess/net](@/docs/api/net.md), [Applications](@/docs/scripting/applications.md#on-the-network).
