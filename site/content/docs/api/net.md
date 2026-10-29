---
title: '@jess/net'
description: 'Tasks on a loop that wait on sockets, name lookups, timers and each other without stopping anything else, and buffered connections, plain or TLS.'
weight: 161
extra:
  kind: api
---

{{ api_signature(value="require('@jess/net'): { loop, run, now, current, spawn, sleep, yield, signal, wait, release, connect, adopt }") }}

The `@dream` network modules never wait: a read with nothing to read says `wouldBlock` and returns. Something has to do the waiting, and if it is a loop around `poller:wait` in the middle of your `update`, everything else in the application waits with it. `@jess/net` is that something, done once. A **loop** owns one poller. **Tasks** are coroutines on it. Every wait a task makes, on a socket, a name lookup, a timer, a signal or another task, yields to the loop, which runs whatever else is ready and comes back when the wait is over or its deadline passes.

An application steps its loop from a host callback, never waiting there. A script or a command target runs one to the end. [`@jess/http`](@/docs/api/http.md) and [`@jess/websocket`](@/docs/api/websocket.md) are written on it, so their requests and sockets wait the same way.

It holds no authority: connecting needs `network.tcp.connect` and resolving `network.dns.resolve`, both the runtime's to grant, and nothing here grants either.

```luau
--!strict
local net = require '@jess/net'

local loop = net.loop()
local ready = net.signal()

loop:spawn(function()
  local notified = ready:wait(net.now() + 1000)
  print(`waiter: notified {notified}`)
end, 'waiter')

loop:spawn(function()
  net.sleep(20)
  print('worker: done')
  ready:notify()
end, 'worker')

print(loop:run(5000))
loop:close()
```

```text
worker: done
waiter: notified true
true
```

## Deadlines

Every wait takes a **deadline**, a time on `net.now()`'s clock in milliseconds, or `nil` for none. A request that must be done in thirty seconds computes `net.now() + 30000` once and hands the same deadline to every step. A deadline doesn't stretch because there are many steps.

A wait that ends returns why: `ready` (or `notified` for a signal), `timeout`, `cancelled` (its task was cancelled), or `closed` (the handle it waited on was closed under it).

Numbers are checked where they come in. A deadline that is NaN raises, as does a span of time that is negative or not finite (`sleep`, `step`, `run`), and a count that is negative, fractional or out of range (`maxConnections`, `maxTasks`, `bufferBytes`, a receive's count or offset): `jess.net.loop: maxTasks is a whole number from 1 to 16777216, not 1.5`. A NaN would never time out, and a negative count has no meaning anyone wants.

## net.loop

{{ api_signature(value="net.loop(options: { maxConnections: number?, maxTasks: number?, onError: ((task: Task, message: string) -> ())? }?) -> Loop") }}

A loop with its own poller. `maxConnections` (256 unless given, at most 16384) bounds the connections open on it at once; `connect` past it fails with `limitReached` instead of exhausting the process's descriptors. `maxTasks` (65536 unless given) bounds the tasks alive on it at once; `spawn` past it raises. It bounds what the loop holds, not how fast work arrives: an application that spawns a task per incoming event still keeps its own bounded queue. `onError` gets each task that raised, instead of `step` raising.

### Loop:spawn

{{ api_signature(value="loop:spawn(body: () -> (), name: string?) -> Task") }}

Starts `body` as a task, first run at the loop's next step. The name appears in failures.

### Loop:step

{{ api_signature(value="loop:step(waitMs: number?) -> number") }}

Runs every task that can run, and returns how many are left. When none can, it waits at most `waitMs` (0 unless given) for a socket to become ready or a deadline to pass, then runs what that woke. A task that raised is handed to `onError`; a loop without one raises after the step, with every failure of the step in one message, so a failure in a task becomes a failure in the host callback that stepped it, reported like any other.

An application calls `loop:step(0)` from `update`. Tasks then run once a frame, without ever holding the frame up, and what arrives between frames waits at most one frame. A step never runs a task more than 64 rounds, so tasks that keep waking each other can't hold a frame forever either.

### Loop:run

{{ api_signature(value="loop:run(timeoutMs: number?) -> boolean") }}

Steps, waiting on the poller between steps, until no task is left (`true`) or `timeoutMs` passes (`false`). For scripts, and for a shutdown handler giving its tasks a last few seconds.

### Loop:close

{{ api_signature(value="loop:close() -> ()") }}

Cancels every task, runs each once more so it can clean up, abandons whatever is still going, closes every connection the loop opened and its poller. Closing twice does nothing. A task can't close its own loop; it cancels itself, or its owner closes the loop from outside.

### Loop:stats

{{ api_signature(value="loop:stats() -> { tasks: number, connections: number, watching: number }") }}

Tasks not yet ended, connections open, and handles the poller watches right now. A handle is watched only while a task waits on it, so an idle loop watches nothing, and a loop that has opened and closed ten thousand connections reports what it holds now, not what it ever held.

## Tasks

{{ api_signature(value="Task: { name: string, status: 'running' | 'done' | 'failed' | 'cancelled', error: string?, cancel: (self) -> (), join: (self, deadline: number?) -> (boolean, string?) }") }}

`cancel` ends the task's current wait with `cancelled`, and every later one at once. The task keeps running until it returns, so it can close what it opened on the way out. A task that hasn't started yet never starts. Everything here that fails for a cancelled wait returns its failure with the kind `cancelled`, so a task that checks what it gets back stops by itself; one that `assert`s on every result raises instead, which is a failure, not a cancellation.

`join`, from another task, waits until this one ends: `true` when it ended `done`, otherwise `false` and its error, `cancelled`, or why the wait ended first.

## net.run

{{ api_signature(value="net.run(body: () -> (), timeoutMs: number?) -> (boolean, string?)") }}

Runs `body` as the only task of a loop of its own, to its end, and closes the loop: `true`, or `false` and the body's error (or `timeout`). It's how a script uses the waits below, and how `@jess/http` serves a call made outside any task.

## net.sleep, net.yield, net.now, net.current

{{ api_signature(value="net.sleep(milliseconds: number) -> boolean") }}

{{ api_signature(value="net.yield() -> boolean") }}

{{ api_signature(value="net.now() -> number") }}

{{ api_signature(value="net.current() -> Task?") }}

`sleep` waits; `false` when the task was cancelled first. `yield` lets every other task run and carries on at the next step. `now` is the monotonic clock deadlines are on, in milliseconds. `current` is the running task, or `nil` outside one.

## net.signal

{{ api_signature(value="net.signal() -> { notify: (self) -> (), wait: (self, deadline: number?) -> (boolean, WakeReason) }") }}

The simplest way for tasks to tell each other something happened. `wait` waits for the next `notify`; `notify` wakes everyone waiting now, and may be called from anywhere, task or not, a host callback included.

## net.spawn

{{ api_signature(value="net.spawn(body: () -> (), name: string?) -> Task") }}

In a task: starts another task on the same loop, as `Loop:spawn` does, for code that has a task but not its loop: a server starting a task per connection, a library fanning out work.

## net.wait

{{ api_signature(value="net.wait(handle: dream_tcp_Watchable, interest: 'read' | 'write', deadline: number?) -> (boolean, WakeReason)") }}

Waits until a `@dream/tcp` listener or stream, a `@dream/tls` session, or a `@dream/dns` lookup is ready. It's the primitive the rest is built on, for protocols that need a raw handle (a listener, a server-side TLS session). Readiness is level-triggered, as the poller's is: try the operation, and wait only when it says `wouldBlock`. One task at a time may wait to read a handle, and one to write it.

## net.release

{{ api_signature(value="net.release(handle: dream_tcp_Watchable) -> ()") }}

Wakes whoever waits on `handle`, with `closed`, and stops watching it, from a task or from anywhere else. Release a raw handle another task may be waiting on before you close it: the wait ends, and the handle can be closed at once. A handle nobody waits on is left as it is. A `Connection` does this itself when it closes.

## net.connect

{{ api_signature(value="net.connect(host: string, port: number, options: ConnectOptions?) -> (Connection?, string?, ErrorKind?)") }}

Resolves `host`, connects to the first address that accepts, and with `tls = true` runs the TLS handshake, verifying `serverName` (the host unless given) against `tlsConfig` (a `@dream/tls` client config; the platform's trust store unless given). `alpn` lists the protocols offered. `deadline` bounds the whole of it, and `bufferBytes` (1 MiB) the received bytes held for the reader.

- **Errors:** returned, as the third value: `resolve`, `connect`, `tls` (the session failed or the certificate didn't verify), `timeout`, `cancelled`, `limitReached`. An option it doesn't take raises.

## net.adopt

{{ api_signature(value="net.adopt(stream: dream_tcp_Stream, options: { tls: dream_tls_Stream?, bufferBytes: number? }?) -> (Connection?, string?, ErrorKind?)") }}

In a task: a `Connection` over a stream already open, such as one a `@dream/tcp` listener accepted, on the running task's loop. With `tls`, a `@dream/tls` session over `stream` whose handshake is done, the connection reads and writes through the session. It counts against the loop's `maxConnections` (`limitReached` past it), and its `host` and `port` are the peer's.

### Connection

{{ api_signature(value="Connection: { host, port, secure, alpn, peerAddress, closed, receive, receiveSome, receiveUntil, receiveInto, buffered, send, close }") }}

A connection buffers what it receives, so a protocol reads in its own units:

- `receive(count, deadline?)`: exactly `count` bytes, as a string.
- `peek(count, deadline?)`: the next `count` bytes, left in place. A protocol peeks a whole unit (a
  frame, a record) before it takes any of it, so a deadline that passes part way through leaves the
  stream where it was, and the next read starts the unit again.
- `receiveSome(maxBytes, deadline?)`: between 1 and `maxBytes`, what is held or the next that arrive.
- `receiveUntil(delimiter, maxBytes, deadline?)`: the bytes before `delimiter`, which is consumed. More than `maxBytes` without it is `limitReached`.
- `receiveInto(target, offset, count, deadline?)`: exactly `count` bytes into a buffer, for bodies that shouldn't become strings.
- `buffered()`: bytes received and not yet taken.
- `idle()`: whether the connection is open and quiet: nobody receiving or sending on it, nothing
  held unread, and nothing arriving on a read that doesn't wait. A pool asks before it reuses a
  connection; a peer that closed it, or sent something nobody asked for, makes it `false`.

Each returns `nil` (or `false`), a message and a kind when it can't: `closed` when the peer ended the stream, `truncated` when a TLS peer ended it without `close_notify`, `timeout`, `cancelled`. One task receives on a connection at a time; a second is a mistake and raises.

`send(data, deadline?)` sends every byte of a string or buffer, or fails. Any number of tasks may send: each send waits for the one before it to finish, so two messages never interleave. A send waits until the socket has taken its bytes and keeps no queue of its own, which is the backpressure: a peer that stops reading stops the sender, not the process's memory.

`close()` closes at once. A task waiting on the connection wakes with `closed`, and a TLS session sends `close_notify` first, as far as the socket takes it.

Related: [@jess/http](@/docs/api/http.md), [@jess/websocket](@/docs/api/websocket.md), [Applications](@/docs/scripting/applications.md#on-the-network), [Capabilities](@/docs/reference/capabilities.md).
