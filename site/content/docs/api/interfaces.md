---
title: 'jess.interfaces'
description: "Look up a domain's interfaces: the plain tables an application's packages provide by id, composed through their overrides."
weight: 35
extra:
  kind: api
---

{{ api_signature(value="jess.interfaces: { get, tryGet }") }}

Also `require('@jess/interfaces')`. An interface is a plain Luau table an [application's](@/docs/scripting/applications.md#interfaces) managed module returns as its `interface`, under the id its manifest gives it. When several provide one id, the last in load order wins, after its `on.host.interfaceOverride` kept what it wanted of the one before. What you get here is that effective table, frozen, in this domain's VM: no proxy, no copy. Interfaces exist once every managed module has loaded, in the application's entry and its handlers. A worker, another domain, `jess run`, and a command target have none.

## jess.interfaces.get

{{ api_signature(value="jess.interfaces.get(id: string) -> any") }}

The effective implementation of `id`.

- **Errors:** `interfaces.get: no package in this domain implements '<id>' (asked at <chunk>:<line>); it has <ids>`; while managed modules are still loading, `interfaces.get: interfaces are composed once every managed module has loaded; look '<id>' up in a handler or in the application's entry`; in a domain no application loaded, `interfaces.get: this domain has no interfaces; an application target composes them from its packages' [managed] modules`; in a worker, `interfaces.get: interfaces are the domain's own code's; a worker has none, so send it what it needs in a message`; outside a domain, `interfaces.get: needs a ScriptDomain`.

```luau
--!strict
type Inventory = {
  getArmorType: (item: string) -> string,
  getWeight: (item: string) -> number,
}

local inventory: Inventory = require('@jess/interfaces').get('game.inventory')
print(`{inventory.getArmorType('cuirass')} cuirass, {inventory.getWeight('cuirass')} pounds`)
```

## jess.interfaces.tryGet

{{ api_signature(value="jess.interfaces.tryGet(id: string) -> any?") }}

The same, or `nil` when no package provides `id`: for an interface that is optional. The other errors are `get`'s; an application that isn't composed yet is a mistake, not an absence.

```luau
--!strict
local achievements = require('@jess/interfaces').tryGet('game.achievements')

if achievements then achievements.unlock('first-death') end
```

Related: [Applications](@/docs/scripting/applications.md#overrides), [jess.events](@/docs/api/events.md).
