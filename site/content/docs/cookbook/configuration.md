---
title: 'Write and Read Configuration'
description: 'Encode a small JSON configuration, preserve the previous file, and validate input after decoding.'
weight: 70
extra:
  kind: guide
---

This script writes a small configuration in the current directory, preserving any previous `settings.json`, then reads and validates it. Run it in a scratch directory first: it changes files.

```luau
--!strict
local fs = require '@dream/fs'
local json = require '@jess/json'
local backup = require '@jess/backup'

local text = json.encodeTable({ mode = 'quiet', retries = 3 }, { indent = 2 }) .. '\n'
local written = backup.write('settings.json', text)
local stored, problem = fs.readFileString(written.path)
if not stored then error(`cannot read settings: {problem}`, 0) end

local decoded = json.decode(stored)
if type(decoded) ~= 'table' then error('settings must be an object', 0) end
if type(decoded.mode) ~= 'string' then error('settings.mode must be a string', 0) end

print(`mode: {decoded.mode}`)
```

Save it as `config.jsl` and run it from that scratch directory:

```sh
jess run --capability filesystem.read --capability filesystem.write config.jsl
```

It prints `mode: quiet`. A second run keeps the first file under an available backup name before replacing it. The JSON decoder returns an unknown value: decoding proves that the text is JSON, not that it has the shape your application expects. This example checks the field it consumes; a real application should validate every required field and define how it handles old versions and invalid input.

The path is relative to the process's current directory, not a prescribed per-user data directory. Choose a location explicitly for the application and its platform. [@jess/backup](@/docs/api/backup.md) documents replacement and failure behavior; [@jess/json](@/docs/api/json.md) documents type mapping and encoder limits. The tested fixture is `tests/fixtures/docs/config.jsl`.
