---
title: '@jess/http'
description: 'Fetch one resource over HTTP or HTTPS: GET and HEAD, bounded in bytes and time, redirects followed under the same rules as the first request.'
weight: 106
extra:
  kind: api
---

{{ api_signature(value="require('@jess/http'): { get, parseUrl, resolveReference }") }}

The client jess's own package distribution reads manifests and downloads archives with, and the one a
tool reaches for when it needs a file from a URL. It is small on purpose: one connection per
request, no cookies, no keep-alive, no pools. It is written over [`@dream/dns`](@/docs/api/_index.md),
`@dream/tcp` and `@dream/tls`, so it holds no authority of its own: the runtime has to hold
`network.dns.resolve` and `network.tcp.connect`, and nothing here grants either. A package target
that fetches declares both; `jess run` takes them with `--capability`.

## get

{{ api_signature(value="http.get(url: string, options: Options?) -> (Response?, string?, ErrorKind?)") }}

Fetches `url` and answers the response, whatever its status, or `nil`, a message and the kind of
failure. A 404 is a response; a host that will not talk is a failure.

- **Parameters:** `options.maxBytes` bounds the body (16 MiB unless given; more is `tooLarge`,
  whether the server declares the length or just keeps sending). `options.timeoutMs` bounds the
  whole request, redirects included (30 seconds). `options.maxRedirects` is how many hops are
  followed (5; `0` returns the redirect as the response). `options.headers` adds request fields;
  `options.method` is `'GET'` or `'HEAD'`; `options.tls` is a `@dream/tls` client config, for a
  trust store other than the platform's.
- **Returns:** `{ status, reason, headers, body, url, redirects }`. Header names are lower-cased
  and repeated fields joined with `, `. `body` is a `buffer`, empty after HEAD or a bodiless
  status. `url` is where the body came from and `redirects` every URL that answered with a
  redirect on the way.
- **Errors:** returned, not raised, as the third value: `invalidUrl`, `unsupportedScheme` (only
  http and https, and never a redirect from https down to http), `credentialsInUrl`, `resolve`,
  `connect`, `tls`, `timeout`, `protocol` (a response that is not HTTP/1.x), `tooLarge`,
  `tooManyRedirects`, `closed` (the connection ended early). An option the function does not take
  raises, as every jess option table does.

```luau
local http = require '@jess/http'

local response, message, kind = http.get('https://dreamweave-mp.github.io/AsHyAmS/network-data/jess-packages-v1.json', {
  maxBytes = 4 * 1024 * 1024,
  timeoutMs = 10000,
})

if not response then error(`{kind}: {message}`) end
if response.status ~= 200 then error(`HTTP {response.status} from {response.url}`) end

local index = require('@jess/json').decode(buffer.tostring(response.body))
```

Every redirect hop is checked exactly as the first request was: same scheme rules, same byte and
time bounds, the same resolution through the OS resolver. Nothing is retried; a caller that wants
a retry policy writes one.

## parseUrl

{{ api_signature(value="http.parseUrl(url: string) -> (Url?, string?, ErrorKind?)") }}

`url` taken apart as `get` reads it: `{ scheme, host, port, target }`, with the scheme and host
lower-cased, the default port filled in, the fragment dropped, and `target` the path and query as
they go on the request line. The same refusals as `get`, without a request.

## resolveReference

{{ api_signature(value="http.resolveReference(base: string, reference: string) -> string?") }}

A `Location` header against the URL it came from, the way a browser resolves it: an absolute URL
as is, `//host/path` under the base's scheme, `/path` on the base's host, anything else against
the base's directory with `.` and `..` folded. `nil` when `base` is not an absolute URL.
