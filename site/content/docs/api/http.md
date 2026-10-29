---
title: '@jess/http'
description: 'HTTP/1.1 requests over HTTP or HTTPS: any method, header fields and body, bounded in bytes and time, redirects followed under the same rules as the first request, without stopping the loop they run on.'
weight: 160
extra:
  kind: api
---

{{ api_signature(value="require('@jess/http'): { request, get, open, client, serve, upgrade, json, parseUrl, resolveReference }") }}

The client `jess workspace sync` fetches manifests and archives with, and the one an application
calls its APIs with. `request` reads a whole response; `open` hands its body out as it arrives;
`client` keeps connections open for the requests after. HTTP/1.1 only, and no cookies. It is
written over [`@jess/net`](@/docs/api/net.md), so a request made in a `@jess/net`
task waits the way the task's other work does, and the loop and every other task on it carry on
while the server thinks. Called anywhere else, a request runs on a loop of its own and returns
when it's done, which is what a script or a command target wants.

It holds no authority of its own: the runtime has to hold `network.dns.resolve` and
`network.tcp.connect`, and nothing here grants either. A package target that fetches declares
both; `jess run` takes them with `--capability`.

## request

{{ api_signature(value="http.request(request: Request) -> (Response?, string?, ErrorKind?)") }}

Sends `request` and answers the response, whatever its status, or `nil`, a message and the kind
of failure. A 404 is a response; a host that will not talk is a failure.

- **Parameters:** `request.url`, an `http://` or `https://` URL. `method`: `GET` unless given,
  and any method token, `PURGE` included. `headers`: request fields, in any case. `body`: a string
  or buffer, sent with its `Content-Length`; `json`: a table sent as JSON instead, through
  [`@jess/json`'s `encodeTable`](@/docs/api/json.md#json-encodetable), with
  `Content-Type: application/json` unless `headers` names another. `maxBytes` bounds the response
  body (16 MiB unless given; more is `tooLarge`, whether the server declares the length, chunks
  it, or just keeps sending). `timeoutMs` bounds the whole request, redirects included (30
  seconds). `maxRedirects` is how many hops are followed (5; `0` returns the redirect as the
  response). `crossOriginRedirects`: `follow` (the default) or `refuse`, what a redirect to
  another origin may do; [Redirects and credentials](#redirects-and-credentials) has the rules.
  `decompress`: ask for gzip and deflate (`Accept-Encoding: gzip, deflate`, unless `headers` names
  one) and decode the body, within `maxBytes`; `false` hands the bytes over as they came.
  `tls` is a `@dream/tls` client config, for a trust store other than the platform's.
- **Returns:** `{ status, reason, headers, body, url, redirects }`. Header names are lower-cased
  and repeated fields joined with `, `. `body` is a `buffer`, empty after HEAD or a bodiless
  status. `url` is where the body came from and `redirects` every URL that answered with a
  redirect on the way.
- **Errors:** returned, not raised, as the third value: `invalidUrl` (also for a space, a control
  character, or a byte past ASCII outside the host: percent-encode them; the request line takes
  the URL as it is), `unsupportedScheme` (only
  http and https, and never a redirect from https down to http), `credentialsInUrl`, `resolve`,
  `connect`, `tls` (the session failed, or the certificate didn't verify), `timeout`, `protocol`
  (a response that is not HTTP/1.x, or frames its body in a way that contradicts itself),
  `tooLarge`, `tooManyRedirects`, `closed` (the connection ended early), `cancelled` (the task
  making the request was cancelled), `limitReached` (its loop holds all the connections it
  allows), `crossOriginRedirect` (a redirect would have carried the body to another origin, or
  left the origin at all under `refuse`).
- **Raises:** for a request that is wrong before anything is sent: an option it doesn't take
  (`unknown jess.http.request option 'bodyy': expected method, headers, body, json, maxBytes, timeoutMs, maxRedirects, crossOriginRedirects, tls`),
  a field the client writes itself (`Host`, `Content-Length`, `Transfer-Encoding`, `Connection`,
  `Upgrade`), a field name that isn't a token or a value with a line break in it, a method that
  isn't a token, `body` with `json`, and a number out of its range: `maxBytes` and `maxRedirects`
  (0 to 20) are whole numbers, `timeoutMs` is finite and above 0.

```luau
--!strict
local http = require '@jess/http'

local response, message, kind = http.request {
  url = 'https://api.example.com/v1/items',
  method = 'POST',
  json = { name = 'lantern', count = 2 },
  headers = { Authorization = 'Bearer <token>' },
  maxBytes = 1024 * 1024,
  timeoutMs = 60000,
}

if not response then error(`{kind}: {message}`) end
if response.status ~= 201 then error(`HTTP {response.status}`) end

local created = http.json(response)
```

The client writes `Host`, `User-Agent` and `Accept` (the last two only when you don't),
`Connection: close`, and `Content-Length` for every request with a body, and for a `POST`, `PUT`
or `PATCH` without one. Your fields go out in name order, so the same request is the same bytes
every time.

What a response may do is bounded, not trusted: a head of more than 64 KiB, a chunk size line of
more than 4 KiB, a `Content-Length` that disagrees with itself (`5, 6`), a field folded over two
lines, and a chunk that isn't followed by CRLF are all `protocol`. `100 Continue` and the other
interim answers are skipped. A body that runs until the connection closes is a body only if a TLS
connection closed with `close_notify`; ended without it, it could have been cut anywhere, and
that is `closed`.

### Redirects and credentials

Every redirect hop is checked exactly as the first request was: the same scheme rules, the same
byte and time bounds, the same resolution through the OS resolver. 307 and 308 repeat the request,
method and body; 303, and 301 or 302 after a POST, go on as a `GET` without the body, as every
client does. Nothing is retried; a caller that wants a retry policy writes one.

A request is written for the origin (scheme, host and port) it was given, and what it carries
stays there. A hop to another origin goes on with only the fields that carry nothing of the
caller's (`Accept`, `Accept-Language`, `User-Agent`, `Cache-Control`, `Range`, `If-None-Match`,
`If-Modified-Since`), and they stay that few for the rest of the chain, even back at the first
origin: `Authorization`, `Cookie`, `X-API-Key` and every other field you set are gone. A hop that
would carry the body to another origin (a 307 or 308 after a POST, say) isn't taken at all: the
request fails with `crossOriginRedirect`, and the other origin hears nothing. A redirect from a
download to a CDN still works, being a bodiless `GET`. An API client that wants no cross-origin
hops of any kind says `crossOriginRedirects = 'refuse'`; one that means to send its fields
somewhere else follows the redirect itself, with `maxRedirects = 0`, and decides.

Secrets stay out of messages. A failure names the URL without its query, where some APIs take
their keys, and a header field that is refused is refused by name, never by value.

A body that came gzip or deflate (zlib, or raw as some servers send it) is decoded, and its
decoded size bounded by `maxBytes` too: a small body that inflates past it is `tooLarge`, never
the heap's problem. `headers` keeps the `Content-Encoding` the server sent. A coding the client
didn't ask for is handed over as it came.

## get

{{ api_signature(value="http.get(url: string, options: Options?) -> (Response?, string?, ErrorKind?)") }}

`request`, with the URL apart from the rest: `http.get(url, { maxBytes = 4096 })` is
`http.request { url = url, maxBytes = 4096 }`, and `options` takes everything `request` does but
`url`.

```luau
--!strict
local http = require '@jess/http'

local response, message, kind = http.get('https://example.com/packages/index.json', {
  maxBytes = 4 * 1024 * 1024,
  timeoutMs = 10000,
})

if not response then error(`{kind}: {message}`) end
if response.status ~= 200 then error(`HTTP {response.status} from {response.url}`) end

local index = http.json(response)
```

## open

{{ api_signature(value="http.open(request: OpenRequest) -> (Stream?, string?, ErrorKind?)") }}

In a `@jess/net` task: the request as `request` makes it, redirects and all, returning as soon as
the final response's head has arrived, with its body still on the connection. A model that
answers token by token, a download too large to hold, an event stream: the body is read as it
comes, and nothing collects it unless you do.

- **Parameters:** `request`'s, but `decompress`: a stream hands out the bytes as they came, so it
  doesn't ask for compression. `maxBytes` bounds the whole body only when given (a stream holds
  nothing, so there is no default); a declared `Content-Length` past it fails `open` at once.
  `timeoutMs` bounds the request until the head arrives. `readTimeoutMs` (30 seconds) bounds each
  `read` that names no deadline of its own.
- **Returns:** a `Stream`: `status`, `reason`, `headers`, `url` and `redirects` as a response has
  them, `ended`, `read` and `close`.

{{ api_signature(value="stream:read(deadline: number?) -> (string?, string?, ErrorKind?)") }}

The next piece of the body: whatever has arrived, up to 64 KiB, as the framing allows (a length,
chunks, or until the connection ends). `nil` with no message at the end, after which `ended` is
true; `nil`, a message and the kind (`timeout`, `closed`, `protocol`, `tooLarge`, `cancelled`)
when it can't be read, after which the stream is ended too.

{{ api_signature(value="stream:close() -> ()") }}

Stops reading. Under a `client`, a stream read to its end gives its connection back for the
next request; one closed early closes it, since the rest of its body is still on the wire. Close
a stream you stop reading: until then its connection is busy.

```luau
--!strict
local http = require '@jess/http'
local net = require '@jess/net'

net.run(function()
  local stream, message, kind = http.open {
    url = 'https://api.example.com/v1/generate',
    method = 'POST',
    json = { prompt = 'hello', stream = true },
    readTimeoutMs = 120000,
  }

  if not stream then error(`{kind}: {message}`) end

  while true do
    local piece, problem = stream:read()
    if not piece then
      if problem then print(`stopped: {problem}`) end
      break
    end

    print(piece)
  end
end)
```

## client

{{ api_signature(value="http.client(options: ClientOptions?) -> Client") }}

A session that keeps its connections open for the next request, for an application that talks to
the same few services over and over. `client:request`, `client:get` and `client:open` take what
`request`, `get` and `open` take but `tls` (the client's own settings serve every connection), and
run in tasks of one loop.

- **Parameters:** `maxConnectionsPerOrigin` (4, at most 64): connections to one origin at once,
  idle or busy; a request past it waits for one to come back, within its `timeoutMs`.
  `idleTimeoutMs` (30 seconds): how long a connection may sit unused before it's closed instead of
  reused. `tls`: a `@dream/tls` client config for every connection.
- **Returns:** a `Client`: `request`, `get`, `open`, `stats` (`{ idle, busy }`), `close`, `closed`.

A connection goes back to the pool only when its response was read to an end its framing
declared, and neither side asked to close it: a body read until the connection ended, an
HTTP/1.0 answer, and `Connection: close` all cost the connection. Before reusing one, the client
checks, without waiting, that the server hasn't closed it in the meantime. If the server closed it
anyway, at the moment the request went out, the request goes again once on a fresh connection,
but only when going twice can't change anything (`GET`, `HEAD`, `PUT`, `DELETE`, `OPTIONS`): a
send that failed part way may already have delivered the request, so a `POST` that met a closed
connection fails, whether it was sending or waiting, and the caller decides.

`client:close()` closes every idle connection at once, and every busy one as its request ends;
anything asked of a closed client fails with `closed`. Nothing about one request stays on the
connection for the next: redirect rules, credentials and limits are each request's own.

```luau
--!strict
local http = require '@jess/http'
local net = require '@jess/net'

net.run(function()
  local api = http.client { maxConnectionsPerOrigin = 2 }

  for _, issue in { 1, 2, 3 } do
    local response = api:request {
      url = `https://api.example.com/issues/{issue}`,
      headers = { Authorization = 'Bearer <token>' },
    }

    if response then print(issue, response.status) end
  end

  print(api:stats().idle)
  api:close()
end)
```

## json

{{ api_signature(value="http.json(response: Response) -> (unknown, string?)") }}

The response body decoded with [`@jess/json`](@/docs/api/json.md), or `nil` and why it isn't JSON.
It decodes whatever came back, so check `status` first; an error page is rarely JSON.

## serve

{{ api_signature(value="http.serve(options: ServeOptions) -> (Server?, string?, string?)") }}

In a `@jess/net` task: listens on `options.address` and serves HTTP/1.1 there until the server is
closed. Every connection is a task of its own on the running task's loop, and the handler runs in
it, so a handler can wait on whatever a task can (a request to another service, a sleep, a
signal) without holding up any other connection, or the frame.

- **Parameters:** `address`, `127.0.0.1:8080` or `[::1]:0` (port 0 takes a free one). `handler`,
  called with each request, returning its response. `tls`, a `@dream/tls` server config, for
  HTTPS. `maxConnections` (256): connections served at once; past it the server stops accepting
  until one ends, so a flood waits in the operating system's backlog instead of in memory.
  `maxHeadBytes` (64 KiB): more is 431. `maxBodyBytes` (16 MiB): more is 413. `headTimeoutMs` (10
  seconds): how long a connection may take to send a whole request head, the wait between
  requests on a kept connection included; past it the connection is dropped without an answer.
  `bodyTimeoutMs` (30 seconds): how long the body may take after the head; past it, 408.
  `maxRequestsPerConnection` (1000). `onError`, called with what a handler raised, or why a
  response couldn't be sent, and the request.
- **Returns:** a `Server`: `address` and `port` as bound, `stats` (`{ connections, requests }`),
  `close` and `closed`; or `nil`, why, and `listen` when the address can't be listened on.
- **Raises:** outside a task, for an option it doesn't take, and, from the runtime, without
  `network.tcp.listen` (and `network.tcp.public` for an address beyond loopback).

The handler gets `{ method, target, path, query, version, headers, body, remoteAddress, secure }`:
field names lower-cased as the client's are, the body read whole within `maxBodyBytes` (by length
or by chunks, `Expect: 100-continue` answered first). It answers with `{ status, headers, body }`,
`json` instead of `body` for a table sent as JSON, or `stream`, a function called with a `send`
that writes one chunk and says whether it went, for a body made as it goes: an event stream, a
long download. The server writes `Date`, `Content-Length` or `Transfer-Encoding`, and
`Connection`, and no body after `HEAD` or with a 204 or 304.

A connection stays open for the next request under HTTP/1.1 unless the client says
`Connection: close`; pipelined requests are answered in order. What could be read two ways, which
is how one request is smuggled inside another, is refused and the connection closed: a request
with both `Content-Length` and `Transfer-Encoding`, a transfer coding other than `chunked` (501),
a `Content-Length` that disagrees with itself, a malformed request line, a field folded over two
lines or holding a control character, an HTTP/1.1 request without exactly one `Host`, and any
version but 1.0 and 1.1 (505). A body that declares more than `maxBodyBytes`, whole or a chunk at
a time, is 413 before any of it is read.

A handler that raises answers 500, and so does a response with a status outside 200 to 599, a
field the server writes itself, or a field with a line break in it; the connection closes, the
failure goes to `onError`, or without one is raised from the connection's task, and the server
carries on serving everyone else. `server:close()` stops listening and closes every connection at
once.

```luau
--!strict
local bytes = require '@dream/bytes'
local http = require '@jess/http'
local net = require '@jess/net'

local secret = 'shared webhook secret'

net.run(function()
  local server, problem = http.serve {
    address = '127.0.0.1:8080',
    handler = function(request: http.ServerRequest): http.ServerResponse
      if request.method ~= 'POST' or request.path ~= '/hooks' then return { status = 404 } end

      local signature = string.match(request.headers['x-signature-256'] or '', '^sha256=(%x+)$')
      if not signature or not bytes.hmacVerify(request.body, secret, 'sha256', bytes.fromHex(signature)) then
        return { status = 401, body = 'bad signature' }
      end

      print(`{request.headers['x-event'] or 'event'}: {buffer.len(request.body)} bytes`)
      return { status = 204 }
    end,
  }

  if not server then error(problem) end

  print(`listening on {server.address}`)
  net.sleep(60000)
  server:close()
end)
```

## upgrade

{{ api_signature(value="http.upgrade(request: UpgradeRequest) -> (Response?, Connection?, string?, ErrorKind?)") }}

In a `@jess/net` task: asks the server to switch the connection to `request.protocol` (sent as the
`Upgrade` field, with `Connection: Upgrade`). On `101 Switching Protocols`, the response and the
open [`Connection`](@/docs/api/net.md#connection), holding whatever the server sent after the
head; the connection is the caller's now. Any other status is the response, its body read within
`maxBytes`, no connection, and the kind `upgrade`. Takes `url`, `protocol`, `headers`, `maxBytes`,
`timeoutMs` (for the connection and the head) and `tls`. Redirects are not followed.
[`@jess/websocket`](@/docs/api/websocket.md) is written on it.

## parseUrl

{{ api_signature(value="http.parseUrl(url: string) -> (Url?, string?, ErrorKind?)") }}

`url` taken apart as `request` reads it: `{ scheme, host, port, target }`, with the scheme and
host lower-cased, the default port filled in, the fragment dropped, and `target` the path and query
as they go on the request line. The same refusals as `request`, without a request.

## resolveReference

{{ api_signature(value="http.resolveReference(base: string, reference: string) -> string?") }}

A `Location` header against the URL it came from, the way a browser resolves it: an absolute URL
as is, `//host/path` under the base's scheme, `/path` on the base's host, anything else against
the base's directory with `.` and `..` folded. `nil` when `base` is not an absolute URL.
