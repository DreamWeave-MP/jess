---
title: '@jess/websocket'
description: 'An RFC 6455 WebSocket client, ws:// and verified wss://: text and binary messages, fragments, pings, the close handshake, and limits on what it will hold.'
weight: 162
extra:
  kind: api
---

{{ api_signature(value="require('@jess/websocket'): { connect }") }}

A WebSocket client, written in Luau over [`@jess/http`'s upgrade](@/docs/api/http.md#upgrade) and
a [`@jess/net`](@/docs/api/net.md) connection. A socket lives on a loop, so everything here runs in
a `@jess/net` task: a socket waiting for its next message waits the way any task does, and the
rest of the loop carries on.

It does the protocol and nothing above it. Pings are answered, messages are reassembled from
their fragments, malformed frames close the connection with the code that names them, and the
close handshake runs both ways. A heartbeat your server expects, sessions, resuming after a drop:
those mean something to an application, and they are the application's to write (the
[Applications](@/docs/scripting/applications.md#on-the-network) guide has one). It offers no
extensions, compression included, so the server can't agree to one, and there is no server side.

Every frame it sends is masked, as RFC 6455 requires of a client, with a fresh key from the
operating system's random source (`@dream/bytes`'s `random`), never from `math.random`.

## connect

{{ api_signature(value="websocket.connect(url: string, options: Options?) -> (Socket?, string?, ErrorKind?)") }}

In a `@jess/net` task: opens a socket to a `ws://` or `wss://` URL, verifying a `wss://` server's
certificate as `@jess/http` does.

- **Parameters:** `options.headers`: fields for the opening request (`Authorization`, `Origin`);
  the handshake's own `Sec-WebSocket-*` fields aren't yours to set. `protocols`: subprotocols
  offered, in order of preference. `timeoutMs`: connecting and the handshake (30 seconds).
  `tls`: a `@dream/tls` client config. `maxMessageBytes`: the largest message `receive` accepts,
  fragments joined (16 MiB). `closeTimeoutMs`: how long `close` waits for the peer's close frame
  (5 seconds).
- **Returns:** the socket, or `nil`, a message and the kind of failure: `@jess/http`'s kinds for
  the URL and the connection, and `handshake` for a server that refused the upgrade or answered it
  wrongly (a status other than 101, a wrong `Sec-WebSocket-Accept`, an extension or a subprotocol
  that wasn't offered).
- **Raises:** outside a task, and for an option it doesn't take.

```luau
--!strict
local net = require '@jess/net'
local websocket = require '@jess/websocket'

net.run(function()
  local socket, message, kind = websocket.connect('wss://echo.example.com/socket')
  if not socket then error(`{kind}: {message}`) end

  socket:send('hello')
  local reply = socket:receive(net.now() + 5000)
  if reply then print(reply.kind, reply.data) end

  socket:close()
end)
```

## Socket

{{ api_signature(value="Socket: { url, protocol, state, closeCode, closeReason, lastPongAt, response, receive, send, ping, close }") }}

`state` is `open` until either side starts the close handshake, `closing` while it finishes, and
`closed` after. `protocol` is the subprotocol the server chose, `response` its answer to the
handshake, and `lastPongAt` when a pong last arrived, on `net.now()`'s clock.

### Socket:receive

{{ api_signature(value="socket:receive(deadline: number?) -> (Message?, string?, ErrorKind?)") }}

The next message, `{ kind = 'text' | 'binary', data = string }`, with text checked as UTF-8.
Control frames are handled on the way and never returned: a ping is answered with its payload,
a pong sets `lastPongAt`. One task receives on a socket at a time.

When the connection closes, `receive` returns `nil`, why, and `closed`, with `closeCode` and
`closeReason` set: the peer's code when it sent a close frame (which is echoed back), 1005 for a
close frame without one, and 1006 when the connection simply ended. A frame that breaks the
protocol closes the connection after telling the peer why, and `receive` returns `protocol` or
`tooLarge`:

| What arrived | Close code |
| --- | --- |
| A masked frame, reserved bits set, a reserved opcode, a control frame fragmented or over 125 bytes, a continuation of nothing, a new message inside a fragmented one, a bad close code | 1002 |
| Text, or a close reason, that isn't UTF-8 | 1007 |
| A frame or message past `maxMessageBytes` | 1009 |

A timeout or a cancelled task leaves the socket as it was: `receive` returns `timeout` or
`cancelled`, and the next call carries on reading.

### Socket:send

{{ api_signature(value="socket:send(data: string | buffer, kind: ('text' | 'binary')?, deadline: number?) -> (boolean, string?, ErrorKind?)") }}

Sends one message as one frame, `text` unless `kind` says `binary`. Any number of tasks may send
on a socket, a heartbeat beside a reply, say; their frames never interleave, and a send waits until
the connection has taken its frame, so a peer that stops reading slows its senders rather than
filling memory. A socket that is closing or closed returns `false` and `closed`. Text that isn't
UTF-8 raises: send other bytes as `binary`.

### Socket:ping

{{ api_signature(value="socket:ping(payload: string?, deadline: number?) -> (boolean, string?, ErrorKind?)") }}

A ping of at most 125 bytes. Its pong arrives through whichever task is receiving, and sets
`lastPongAt`.

### Socket:close

{{ api_signature(value="socket:close(code: number?, reason: string?) -> ()") }}

Starts the close handshake with `code` (1000 unless given; 1000 to 1014 but 1004 to 1006, or 3000
to 4999) and a reason of at most 123 bytes of UTF-8, waits up to `closeTimeoutMs` for the peer's
close frame, and closes the connection. If another task is receiving, that task reads the peer's
close and ends with `closed`; if none is, `close` reads for it. Closing a closed socket does
nothing.

Related: [@jess/net](@/docs/api/net.md), [@jess/http](@/docs/api/http.md), [Applications](@/docs/scripting/applications.md#on-the-network).
