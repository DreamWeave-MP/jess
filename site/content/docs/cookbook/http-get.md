---
title: 'Fetch a Resource over HTTP'
description: 'Make a bounded GET request and handle transport failures separately from HTTP status codes.'
weight: 50
extra:
  kind: guide
---

This script fetches one URL supplied on its command line. Save it as `fetch.jsl`:

```luau
--!strict
local http = require '@jess/http'
local url = ...
if type(url) ~= 'string' then error('expected a URL', 0) end

local response, message, kind = http.get(url, { maxBytes = 4096, timeoutMs = 5000 })
if not response then error(`{kind}: {message}`, 0) end
if response.status ~= 200 then error(`HTTP {response.status}`, 0) end

print(buffer.tostring(response.body))
```

Run it against a server you choose:

```sh
jess run --capability network.dns.resolve --capability network.tcp.connect fetch.jsl https://example.com/
```

The response body goes to stdout. It is bounded to 4 KiB and the whole request, including redirects, to five seconds. A network or protocol failure returns no response and an error kind; an HTTP error status is still a response and this script handles it separately. The script prints the body as text, so it is appropriate for text responses rather than arbitrary binary data.

This example requires network access and a reachable HTTP or HTTPS server. For HTTPS it also needs a working platform trust store. Its fixture, `tests/fixtures/docs/http-get.jsl`, is exercised against a loopback server by the integration tests; those tests do not contact the internet. The [HTTP reference](@/docs/api/http.md) describes redirects, headers, bounds, and errors. This API provides GET and HEAD; it is not a general WebSocket or HTTP server API.
