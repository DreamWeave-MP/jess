---
title: '@jess/http'
description: 'HTTP/1.1 requests over HTTP or HTTPS: any method, header fields and body, bounded in bytes and time, redirects followed under the same rules as the first request, without stopping the loop they run on.'
weight: 160
extra:
  kind: api
---

{{ api_signature(value="require('@jess/http'): { request, get, upgrade, json, parseUrl, resolveReference }") }}

The client `jess workspace sync` fetches manifests and archives with, and the one an application
calls its APIs with. It is small on purpose: one connection per request, no cookies, no keep-alive,
no pools. It is written over [`@jess/net`](@/docs/api/net.md), so a request made in a `@jess/net`
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
  response). `tls` is a `@dream/tls` client config, for a trust store other than the platform's.
- **Returns:** `{ status, reason, headers, body, url, redirects }`. Header names are lower-cased
  and repeated fields joined with `, `. `body` is a `buffer`, empty after HEAD or a bodiless
  status. `url` is where the body came from and `redirects` every URL that answered with a
  redirect on the way.
- **Errors:** returned, not raised, as the third value: `invalidUrl`, `unsupportedScheme` (only
  http and https, and never a redirect from https down to http), `credentialsInUrl`, `resolve`,
  `connect`, `tls` (the session failed, or the certificate didn't verify), `timeout`, `protocol`
  (a response that is not HTTP/1.x, or frames its body in a way that contradicts itself),
  `tooLarge`, `tooManyRedirects`, `closed` (the connection ended early), `cancelled` (the task
  making the request was cancelled), `limitReached` (its loop holds all the connections it
  allows).
- **Raises:** for a request that is wrong before anything is sent: an option it doesn't take
  (`unknown jess.http.request option 'bodyy': expected method, headers, body, json, maxBytes, timeoutMs, maxRedirects, tls`),
  a field the client writes itself (`Host`, `Content-Length`, `Transfer-Encoding`, `Connection`,
  `Upgrade`), a field name that isn't a token or a value with a line break in it, a method that
  isn't a token, and `body` with `json`.

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
client does. `Authorization`, `Proxy-Authorization` and `Cookie` go only to the origin (scheme,
host and port) they were given for: a hop to anywhere else drops them, and they stay dropped for
the rest of the chain. Nothing is retried; a caller that wants a retry policy writes one.

Secrets stay out of messages. A failure names the URL without its query, where some APIs take
their keys, and a header field that is refused is refused by name, never by value.

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

## json

{{ api_signature(value="http.json(response: Response) -> (unknown, string?)") }}

The response body decoded with [`@jess/json`](@/docs/api/json.md), or `nil` and why it isn't JSON.
It decodes whatever came back, so check `status` first; an error page is rarely JSON.

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
