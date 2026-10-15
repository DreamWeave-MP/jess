# Runtime decisions

The contract fixtures pin what scripts observe of the runtime hosting jess. They were first blessed under mlua 0.12.1 (Luau 0.736), so the port to l3i 1.0.0 (Luau 0.740) could be held to them; entries 1 to 19 are the oddities found while writing the fixtures, entries 20 on are what the port changed on purpose. Each entry says what it is, whether it's compatibility (scripts may rely on it; keep it) or a bug (don't carry it forward), what was decided, and how the fixtures encode that decision.

A fixture never pins a bug. Where current behavior is a bug, the fixture pins only the part that's right, or nothing, and the entry says so. Where behavior is compatibility but dubious, `questionable/` pins it so that changing it is a deliberate act: update the fixture and the inventory, and say why in the commit.

## Error text

### 1. Error messages name mlua's value kinds

- **What:** jess's own messages name a value's kind with mlua's `type_name()`, so a whole number is an `integer`: `openmw must be a boolean or an OpenMW context, not integer`. Luau has no integer type; `type(1)` is `number`.
- **Class:** bug (cosmetic; a leak of the host library into the script's view).
- **Decision:** fixed. jess's messages name a value's kind as `type()` does (`jess_lua::kind`: `number`, and `userdata` for `jess.null`), and the fixtures that stopped before the kind word pin the whole message (`not number`).

### 2. Argument conversion errors are the runtime's

- **What:** an argument of the wrong type that jess declares as a Rust type fails in mlua's conversion, with mlua's text: `bad argument #2 to 'LuaPlugin.record': error converting Lua integer to usize (out of range)`. It names Rust types and mlua's internal method names.
- **Class:** implementation detail.
- **Decision:** record indices (`Plugin:record`, `Plugin:insert`'s position, `Nif:record`, the `index` dump option) are converted by jess: `expected a record index (a whole number from 0), not -1`, pinned in `plugin/records` and `questionable/coercion`. So are string and list-of-strings option values (`encoding must be a string, not table`, pinned in `options/unknown`). For the conversions still left to the runtime (a missing path, a table where a handle goes), only the category is the contract: fixtures assert `conversion` with an empty needle.

### 3. Deleted records fail with two different messages

- **What:** a handle to a deleted plugin record fails with `record was deleted`; one to a deleted NIF record with `this record has been deleted`.
- **Class:** compatibility (both say the right thing; scripts may match either).
- **Decision:** both pinned as substrings (`plugin/records`, `meta/meta`, `nif/edit`). Unifying them is fine, as a deliberate change to those fixtures.

## Arguments

### 4. Unknown option keys are mostly ignored

- **What:** most option tables skipped keys they don't take, so a typo did nothing: `plugin:save(path, { bakup = false })` still backed up, `jess.plugin.open(path, { tpyes = ... })` opened everything. Selector specs, `cell`, `Nif:shape`, and record fields rejected unknown keys.
- **Class:** bug: a misspelled option failed silently, which NEXT.md §59 rules out ("do not silently ignore unsupported types/values").
- **Decision:** fixed. Every option table rejects a key it doesn't take, through one check (`jess_lua::check_options`): `unknown Plugin:save option 'bakup': expected backup`. `options/unknown` pins every table's message, `inventory.toml` marks every table `rejected`, and `the_option_tables_reject_the_keys_they_dont_take` checks each message's keys against the inventory's. The selector's message says `option` now, not `key`, and `Nif:shape`'s names the method.

### 5. Arguments are coerced

- **What:** mlua converts where the type doesn't match: a number is accepted as a path or id (`jess.fs.exists(12)`), a numeric string as an index (`plugin:record('1')`), and a fractional index is truncated (`plugin:record(1.5)` is record 1). A negative index is a conversion error.
- **Class:** compatibility for number/string coercion (Lua scripts expect it); bug for truncating fractional indices.
- **Decision:** number/string coercion stays, pinned in `questionable/coercion`. Fractional truncation is fixed: a fractional record index is an error, like a fractional view index (`lists are indexed by whole numbers`, `plugin/views`), and `questionable/coercion` pins it for plugin and NIF records.

## Records

### 6. A deleted NIF record's index survives in decoded links

- **What:** after `record:delete()`, a record that linked to it reads the link as `nil` through its view, and saving writes an empty link, but `decode()` (and `nif:dump()`) still give the deleted record's index. Writing that value back (`record:set(record:decode())`, or `edit` with no change) then fails with `there's no record 3 to link to`.
- **Reproduce:** open `random_objects.nif`, delete `Empty 1` (record 3), then `nif:record(0):decode().children` still lists `3`.
- **Class:** bug. A decoded link to a missing record should be `jess.null`, as `NifDump.roots` already spells an empty root.
- **Decision:** fixed. `decode`, `edit`, `toTable`, and `nif:dump` copy such a link as `jess.null` (`nil` for a single link), as views read it; `nif/edit` pins all four and writes the decoded record back (`deleted.decoded`).

### 7. A CELL opened without its header reports a reference's id

- **What:** with `jess.plugin.open(path, { cell = { header = false } })`, `jess.meta(cell).id` is the id of the cell's first reference (`test_npc` in `all_types.esp`), because the first `NAME` left in the record is the reference's.
- **Class:** bug. The cell has no id to give without its header; it should be `nil`.
- **Decision:** fixed. A CELL parsed without its header has no id (`omwaddon::records::record_id`, `RecordView::id`), so `jess.meta(cell).id` is `nil` and `find` doesn't match a reference's id; `plugin/load` pins both (`cellReferences`).

### 8. The save header's readable record is snake_case

- **What:** `save:values()` gives the `SAVE` record's readable shape with snake_case keys (`format_version`, `in_game_time`, `game_hour`, `time_played`, `screenshot_bytes`), while `save.header`, the same data, is camelCase, as every other table the API hands out is.
- **Class:** bug (the Luau API renames struct fields to camelCase; this one slips through because `values()` copies the CLI's JSON keys as they are).
- **Decision:** fixed. `values()` camelCases field names as `Record:display()` does, except in a save-only record's tag-keyed `subrecords` (whose `LUAD` Lua data keeps its own keys); `save/openmw` pins the header's whole value. `jess save dump` keeps its own keys.

### 9. Typed record handles promise iteration they don't have

- **What:** the definitions type a record handle as `...Record = RecordHandle & ...View`, and every `...View` includes `ViewIteration` (`__iter`, `__len`), but a record handle registers neither: only views (a record's structs and lists) iterate and count. `inventory.toml` lists the handle's metamethods as `__eq`, `__index`, `__newindex`, `__tostring`.
- **Class:** bug, in the definitions or the handle; which one is a design call.
- **Decision:** fixed, in the definitions: a record handle's view type (`LightView`, `NiNodeView`) leaves out `ViewIteration` (`jess_luau_types::with_record_views`), so only the structs and lists inside a record promise it. Iterating a record's fields goes through `record:toTable()`.

### 10. A NIF record's decoded table has no `index`

- **What:** the NIF record types declare `index: number?`, which `jess.nif.dump` fills in; `record:decode()` leaves it out.
- **Class:** compatibility (the field is optional, and a record's number is `jess.meta(record).index`).
- **Decision:** pinned as absent (`meta/invariant`, `nif.decodedIndex`).

### 11. A NIF without a path dumps with `path = ""`

- **What:** `nif:dump()` on a NIF from `jess.nif.new()` or `openBytes` gives `path = ""`, while `nif.path` is `nil`.
- **Class:** compatibility (`NifDump.path` is typed `string`).
- **Decision:** pinned (`nif/build`).

### 12. `jess.land.write` doesn't type-check what `jess.land.read` returns

- **What:** `LandParts`, what `write` takes, types each part `(buffer | false)?`; `LandTerrain`, what `read` returns, types them `buffer?`. Table properties are invariant for the checker, so `jess.land.write(record, jess.land.read(record))` fails to type-check, though it runs (commit `29deab3` says a `LandTerrain` already fits; luau-analyze disagrees).
- **Class:** bug, in the definitions.
- **Decision:** fixed. `write` takes a `LandInput`, `LandParts` with read-only fields (it only reads them), so the checker compares them covariantly and both a built `LandParts` and a `LandTerrain` fit. `land/land` writes the read terrain back with no cast.

### 13. A vector writes three numbers, but the definitions don't say so

- **What:** writing a Luau `vector` into a field of three numbers (`node.translation = vector.create(1, 2, 3)`) works: the runtime turns it into a list. The definitions type such fields as `{ number }`, so typed code has to cast.
- **Class:** compatibility at runtime, gap in the definitions.
- **Decision:** the runtime behavior is pinned (`nif/edit`, `fromVector`); the fixture casts, with a comment pointing here.

## Configs and require

### 14. Activation is case-sensitive

- **What:** `context:activate('mine.esp')` followed by `context:activate('Mine.esp')` returns `true` twice, and `save()` writes both `content=` lines. OpenMW's own check for repeated content is case-sensitive too (`apps/openmw/main.cpp`, `contentDedupe`), but it finds content files ignoring case (`components/files/collections.cpp`, `Collections::getPath`), so the result loads one plugin twice.
- **Class:** bug (jess knows the names are the same file).
- **Decision:** fixed. `OpenMwContext::activate_content` and `deactivate_content` (shared by `context:activate`, `context:deactivate`, and `jess openmw enable`/`disable`) match an active name ignoring ASCII case; `openmw/context` pins `activation.otherCase` as `false`, one `content=` line, and deactivating by another spelling.

### 15. Relative `require` resolves against the calling function

- **What:** `./` and `@self/` names resolve against the chunk of the function that calls `require`. Called straight from `pcall(require, './x')`, the caller is `pcall`, so there's no script to be relative to: `module not found: ./x: a relative require needs a script file to be relative to`.
- **Class:** compatibility (Luau's require-by-string also resolves against the requiring chunk).
- **Decision:** pinned (`downstream/require`, `errors.throughPcall`).

### 16. `jess.fs.write` doesn't back up unless asked

- **What:** `jess.fs.write` backs up an existing file only with `{ backup = true }`; `Plugin:save`, `Nif:save`, `jess.save.strip`, and `OpenMwContext:save` back up unless told not to.
- **Class:** compatibility (documented: `fs.write` is plain file access).
- **Decision:** pinned (`fs/fs`, `plugin/save`, `nif/edit`, `save/files`, `openmw/context`).

## Values

### 17. TOML integers past 2^53 lose precision

- **What:** `jess.toml.decode` read integers as Luau numbers, so `9007199254740993` read as `2^53`.
- **Decision:** fixed by removal. jess reads TOML with the built-in package's `@jess/data/toml`, which
  keeps an integer past 2^53 exact as an l3i `integer`; `jess.toml` and its fixture are gone.

### 18. `__tostring` formats

- **What:** handles describe themselves for printing: `Plugin(<path>, 80 records)`, `RecordMeta(LIGH #36)`, `NiNode 0 "Root"`, `Save(Tester, level 3)`, `ObjectRef(1, 2)`. A view's is Rust's debug output of its path (`view [Field("data"), Index(0)]`).
- **Class:** compatibility for the handles' (designed, readable); implementation detail for the view's.
- **Decision:** the handles' are pinned where a fixture prints them; the view's is not.

### 19. `typeof` names userdata by their Rust types

- **What:** `typeof(plugin)` is `LuaPlugin`, `typeof(record)` is `LuaRecord`, and Luau's own errors say so (`attempt to iterate over a LuaRecord value`). The definitions call them `Plugin` and `Record`; `type()` says `userdata`.
- **Class:** implementation detail.
- **Decision:** not pinned. Fixtures use `type()`, and describe values through the API, never through `typeof`.

## The l3i port

### 20. The libraries hand out views, not tables

- **What:** vfstool's `keys()`, `entries()`, and the other lists, and openmw-config's `contentFiles()` and setting rows, are sequence or stream views on l3i: `#`, `[i]`, `for _, x in view`, and `:toTable()`; `ipairs`, `pairs`, and `table.clone` over them fail. Indices are 1-based (vfstool's used to be 0-based), sizes and hashes are Luau integers, option tables are strict, and errors read `dream.<ext>: <context>: <message>`. Each crate's own docs and the plan's `.d.jsl` are the contract.
- **Class:** deliberate break, made upstream.
- **Decision:** pinned as views. `downstream/libraries` and `openmw/context` read them through `oracle.plain`, which copies a view into a table for the JSON only, so the fixtures exercise the view the way a script does. `inventory.toml` lists vfstool's `host` table, which jess's runtime grants (`filesystem.host`), and `[declared]` names the plan's module types (`Module__dream_vfs`).

### 21. `collectgarbage` and `loadstring` are gone

- **What:** l3i opens Luau's standard libraries without `collectgarbage` (the host drives the collector) and without `loadstring`.
- **Class:** deliberate break, made upstream.
- **Decision:** dropped from `inventory.toml [globals]`. Nothing jess ships called either.

### 22. Errors are messages

- **What:** under mlua, an error jess raised reached Luau as an error object whose text carried a stack traceback, which `pcall`, `xpcall`, and `coroutine.resume` were shimmed to strip. l3i raises jess's errors as plain strings, so the shim is gone and every path (`pcall`, a callback into Lua, the host's report) sees the message alone. `lua_api`'s uncaught-error test asserts the message instead of a traceback.
- **Class:** fix (the shim existed to hide a host library's shape).
- **Decision:** the fixtures were already asserting messages; nothing changed in them.

### 23. `jess.null` is a null light userdata

- **What:** as under mlua: `type(jess.null) == "userdata"`, equal to itself, unequal to everything else. Lists jess writes carry a metatable, jess's array marker, so an empty list stays a list when it's read back (tes3merge's `isArray` relies on `getmetatable(value) ~= nil`).
- **Class:** compatibility.
- **Decision:** kept; the marker is a frozen empty table per runtime.

### 24. Record handles keep their methods in a table

- **What:** l3i's planner generates a handle type's `__index` from its declared members, and a type with an explicit `__index` can declare none. A plugin record, NIF record, or view handle reads every name as a field of its record, so its `__index` is explicit, and its methods live in a per-runtime table it consults first. `LuaDataFixed`'s `kind`, `values`, and components are read the same way.
- **Class:** implementation detail; the surface is unchanged.
- **Decision:** `inventory.toml` lists the methods as before and `LuaDataFixed`'s fields as none. `namespaces.rs` reads the plan's members plus the method tables.

### 25. Option tables and trailing arguments may be left out

- **What:** l3i's binder is fixed-arity and validates argument counts before conversions; mlua filled missing trailing arguments with `nil`. Every option table (`Opts`) and every argument a script may omit (`plugin:insert(record)`, `plugin:select()`, `jess.land.read(record)`, `jess.nif.check()`, `nif:fix()`, `jess.meta()`) is declared optional, and the messages for the arguments jess checks itself (`expected a buffer or string, not table`, `expected a record index ...`, `jess.meta takes ... not nil`) are jess's, carried inside l3i's `bad argument` wording.
- **Class:** compatibility.
- **Decision:** the fixtures pin the same messages as before.

### 26. serde's error wording is kept

- **What:** `serde_value`'s own deserialize errors read `Missing field weight`; the fixtures pin serde's `missing field \`weight\``.
- **Class:** compatibility.
- **Decision:** jess deserializes through its own error type, whose `custom` is all serde needs to spell every error the standard way (`convert::DeserializeError`).

### 27. `typeof` names userdata by their l3i names

- **What:** `typeof(plugin)` is `jess.Plugin`, `typeof(record)` is `jess.Record` (it was `LuaPlugin` and `LuaRecord`); `type()` still says `userdata`.
- **Class:** implementation detail (#19).
- **Decision:** not pinned, as before.

### 28. The safe-environment canary's floor is 3x

- **What:** `tools/bench/canaries/safeenv.jsl` compares a loop in the script's environment with one moved into an unsafe copy. mlua's Luau build measured the safe one over 5x faster; l3i's build runs the unsafe copy faster, so the same safe environment measures 4x to 5x, and one not marked safe about 1x.
- **Class:** measurement, not behavior.
- **Decision:** the floor is 3x, which still fails the moment the globals aren't marked safe.

### 29. Scripts load on the thread that runs them

- **What:** l3i 1.0's script templates loaded on a loader thread whose globals were empty and unsafe, so Luau couldn't pre-resolve builtin imports at load time (`lvmload.cpp`, `ResolveImport`); the CLI's one global environment wants them resolved. jess compiles each chunk once per runtime (by chunk name) and loads the bytecode on the running thread. On l3i 1.0 that was a hand-rolled `luau_load`, and a module reached through `require` ran interpreted because the code generator was only reachable from the `Runtime`; since l3i 1.1.0 it is `LoadScope::load_bytecode`, which loads on any scope's thread and compiles natively under the plan's policy, so a required module is as native as the script that required it.
- **Class:** implementation detail; the l3i gap it worked around is closed upstream.
- **Decision:** none visible to scripts.

### 30. Every script the host runs compiles to native code

- **What:** mlua's `luau-jit` feature ran `luau_codegen_compile` on every chunk it loaded, so the mlua benchmark baselines are native code throughout; l3i's `NativeCodeMode::Annotated` compiles only `--!native` chunks, and the `safeenv` canary measured 0.135 s against the baseline's 0.022 s until it was annotated (0.015 s). The plan runs `NativeCodeMode::Eager`: every chunk `run_scripts`, `run_file`, `modify_plugin`, and `modify_nif` load is compiled natively, annotated or not. Modules a script `require`s still aren't (#29).
- **Class:** measurement parity; the same behavior users had.
- **Decision:** `Eager`, recorded here and in the plan's builder.

### 31. An object reference is one Luau integer

- **What:** `ObjectRef` was a userdata with `index` and `contentFile` fields and an `ObjectRef(1, 2)` `__tostring`; a save's `luaData()` allocated one per reference. It's now a Luau `integer` holding the refnum (index in the high 32 bits, content file in the low 32): `jess.luadata.objectRef(index, contentFile)` returns it, `refIndex(ref)` and `refContentFile(ref)` read it, `==` compares it, and `ObjectList[i]` gives one. Encoding takes any Luau integer in Lua data as a reference, since numbers there are always floats.
- **Class:** deliberate break (the handoff's step 6.4): zero allocation per reference, and a value `--!strict` scripts can't confuse with a count.
- **Decision:** `luadata/luadata` re-blessed; `ObjectRef` in the definitions is `integer`; `[userdata.ObjectRef]` left the inventory.

### 32. A deleted record's `meta.index` is nil

- **What:** `jess.meta(record).index` was a getter that raised `record was deleted` once the record was gone. It's an l3i direct field now, served into the destination register without a call, so tes3merge's per-record filter matching (`matches[jess.meta(record).index]`) costs an indexed load; a direct field can't raise, so a deleted record reads `nil`, the one metadata that answers instead of raising. `size` stays a getter: it needs the record's bytes, and an edit that can't be encoded has to be an error there, not a `nil`.
- **Class:** deliberate break, for speed; a deleted record has no number, and `nil` says so.
- **Decision:** `plugin/records` re-blessed (`metaIndex` observes `nil`); the definitions keep `number` and say so in the doc comment, as `tag: string` does.

## Shaders

### 33. A number in a shader is a `Float`

- **What:** a Luau number has no `Float`/`Int`/`UInt` type, and `@jess/shader` takes one wherever a shader value goes (`shader.max(facing, 0)`, `2 * color`, `shader.vec4(position, 1)`). It's a `Float` literal, written at the operation's line. `Int` and `UInt` have no spelling yet: nothing the acceptance shaders do needs one, and the IR has no integer vectors.
- **Class:** design.
- **Decision:** pinned in `shader/program` (`numberFirst`, `splat`, `constant`). Integer constants arrive with a consumer, as `shader.int` and `shader.uint`, not by guessing from a number's value.

### 34. A texture comes with its sampler

- **What:** the shader handoff writes `shader.sample(input.diffuse, input.diffuseSampler, input.uv)`, the render bible `shader.sample(input.diffuse, input.uv)`; the IR needs an explicit `Sampler`. `textures = { diffuse = shader.texture2d }` declares a `Texture2D` and a `Sampler` both named `diffuse` (they lower to `t_diffuse` and `s_diffuse`), and `sample(texture, uv)` pairs them. A sampler declared on its own (`parameters = { point = shader.sampler }`) goes between: `sample(texture, sampler, uv)`. The two forms differ by argument count, never by probing what an argument is.
- **Class:** design.
- **Decision:** both pinned in `shader/program` (`diffuse`, `explicitSampler`); a two-argument `sample` of anything but a declared texture is an error saying so (`shader/errors`, `sampler`).

### 35. Shader errors name the line, never a column

- **What:** Luau's debug info records lines. A shader value's origin is the chunk and line of the nearest Luau function on the call stack (so a call through `pcall` still points at Luau source), and the module that called `shader.program` as its owner: `shaders/lit.jsl:12 (shaders/lit.jsl): dot(Vec3f, Vec4f) is invalid: expected two float vectors of the same size`. A mistake only the whole program shows (a `Vec3f` position, a texture sampled in the vertex stage) is reported when the program is finished, every problem on its own line under `<owner>: the shader program is invalid`.
- **Class:** design.
- **Decision:** pinned whole in `shader/errors` (`operations`, `program`), line numbers included: moving a line in that fixture is a re-bless.

### 36. `shader.program` validates and doesn't fold

- **What:** the program a module returns is validated and otherwise as written. Constant folding (`jess_shader::Program::fold_constants`) is what the consumer does before lowering: it rewrites the program, and rewriting by default is what jess doesn't do.
- **Class:** design.
- **Decision:** nothing a script observes; `tests/shader.rs` folds before it lowers.

### 37. jess's modules are `@jess/<name>`

- **What:** `require("@jess/shader")` is `jess.shader`, because a shader module starts with that line. So is every other module table (`@jess/plugin` is `jess.plugin`); the bundled scripts' names (`@jess/doctor`) aren't modules and still aren't requirable. `declare require` types `@jess/shader` and leaves every other name `any`.
- **Class:** addition.
- **Decision:** pinned in `shader/program` (`sameModule`); `[modules."jess.shader"]` lists the members.

### 38. `@jess/log` and `@jess/test` are built-in modules written in Luau

- **What:** `require("@jess/log")` and `require("@jess/test")` are modules jess ships, embedded in the binary, installed through the runtime plan like the native `@dream/*` modules (`jess_lua::stdlib`), frozen, and typed as `Module__jess_log`/`Module__jess_test` in the definitions. `@jess/` is jess's namespace: a `@jess/` name that isn't one of jess's members or built-in modules is `module not found`, never a VFS or package lookup, so no file can stand in for one. `@jess/log` writes through one native sink the host selects (`set_log_sink`); scripts can't reach it.
- **Class:** addition.
- **Decision:** `[modules."@jess/log"]` and `[modules."@jess/test"]` list the members, probed through `require` (`surface.jsl` walks the inventory's `@` paths); `[declared]` names their types. `tests/stdlib.rs` pins freezing, the private sink, the closed namespace, and that a data folder's `jess/log.lua` or a package's own `jess/log.jsl` doesn't shadow them.

## Domains

### 39. `jess.input` and `jess.l10n` bind to the domain's services, and check capabilities only there

- **Superseded for `jess.l10n`** by 47: localization left the native API; what follows holds for `jess.input`.

- **What:** under a `ScriptDomain` that carries a `Mutex<jess_input::InputService>` or a `RwLock<jess_l10n::Registry>` service (jess-host attaches both), `jess.input.runtime()` is the application's live input (the host ticks it; `tick` and `advance` are refused) and every `jess.l10n` call reaches the application's registry; anywhere else each runtime keeps a headless runtime and a registry of its own, which is what the fixtures drive. Every runtime method and every l10n call checks its capability (`input.actions`, `input.observe`, `input.configure`, `l10n.read`, `l10n.configure`) through the broker when the script runs under a domain; outside any domain nothing holds authority and nothing is checked, since the process's own user is running the script.
- **Class:** addition. No fixture changed: the fixtures run outside any domain, where the API is what it was.
- **Decision:** pinned in `lua_api` (`input_and_l10n_calls_are_denied_without_their_capabilities_under_a_domain`, `a_domain_registry_is_shared_with_the_workers_the_host_shares_it_with`, `a_domain_input_service_is_the_live_runtime_and_the_host_ticks_it`) and in the kernel fixture; the denial is the broker's, `capability 'input.actions' denied to domain#1 gen#1 context#2`, as `pcall` reads it.

### 40. A worker shares the domain's services only when its spawner says `shareServices = true`

- **Since 47** no service carries localization: a worker's `@jess/l10n` is always its own.

- **What:** `worker.spawn` takes `shareServices`, a boolean: `true` gives the worker every service its domain carries (`WorkerSpec::share_all`), so `jess.l10n` in it formats from the application's registry; absent or `false`, the worker has none and its `jess.input.runtime()` and `jess.l10n` are its own. A script can't name a Rust type, so the choice is all or nothing; the worker's capabilities still say what it may do with what it sees.
- **Class:** addition. The unknown-option message names the new option.
- **Decision:** `options/unknown` re-blessed (`expected capabilities, name, shareServices, or source`); pinned in the kernel fixture (`greeter` reads the shared registry and is denied input).

## Files

### 41. `jess.fs.replace` writes a file through a temporary file and a rename

- **What:** `jess.fs.write` truncates the file and writes into it, so a crash or a full disk leaves it half-written. `jess.fs.replace(path, data)` writes to `.<name>.jess-<pid>-<n>.tmp` beside the target, syncs it, renames it over the target, and syncs the folder, keeping the old file's permission bits (`jess_plugin::files::replace_file`); a failure removes the temporary file and leaves the target as it was. It doesn't create the folder: a rename can't. The `openmw-import-morrowind` package writes every openmw.cfg through it, the way dream-ini's `write_atomic` wrote them; it is general because that is the shape of any config file another program reads.
- **Class:** addition.
- **Decision:** pinned in `fs/fs` (`replaced`, `replaceMissingFolder`); `replace` joins `[modules."jess.fs"]`, and `missing: failed to write` the `[errors]`.

## The Morrowind.ini importer

### 42. The `openmw-import-morrowind` port is held to goldens frozen from the dream-ini library

- **What:** the package's differential suite (`tools/jpk/openmw-import-morrowind/tests/differential.jsl`) ran every case through the native `dreamIni` binding and the port side by side. It now reads the library's answers from `tests/golden/<function>.jsonl`, one answer per call in the suite's order, each with a fingerprint of the call's input and the generators' seeds and counts in the header, handed over by the Rust harness (`libs/lua/tests/ini_experiment.rs`) as the `oracle` global. `JESS_BLESS=1` makes the harness answer from the dream-ini library itself, through the same conversions the binding made, and write what it said, so a changed suite or generator is re-blessed, never silently compared against the wrong case. The binding's own option messages (`ini.importMaps.verbose: expected boolean, got string`), which no library call produces, are pinned in the suite as text, captured from the binding while it was still in the plan. The library stays a dev-dependency of `jess-lua` for that regeneration, for the writers' byte-for-byte differential, and for the bench's Rust baseline; nothing jess ships links it.
- **Class:** test infrastructure; the goldens are what the native binding answered, since the port matched it on every case the day they were blessed.
- **Decision:** `tests/golden/{parseIni,parseCfg,serializeCfg,importMaps,importPaths}.jsonl`, rewritten only with `JESS_BLESS=1` and a reason.

### 43. `dreamIni` and `@dream/ini` left the runtime

- **What:** jess no longer links dream-ini. The `dreamIni` global, `require("@dream/ini")`, `require("dreamIni")`, and the `Module__dream_ini` type are gone from the plan, the compat globals, and `jess luau-types`. What replaces them is the `openmw-import-morrowind` package (`tools/jpk/openmw-import-morrowind`), the port of the same library to Luau: `parseIni`, `parseCfg`, `serializeCfg`, `importMaps`, `importPaths`, `version`, and the same warnings, events, options, and messages, held to the library's answers by its goldens (#42); plus `readPluginHeader` and the writers the crate never exposed to Luau (`saveCfg`, `saveResolvedCfg`, `loadDocument`, `applyUpdate`, `renderDocument`, `saveDocument`, `saveResolvedDocument`). The package is loaded from disk with `jess run` or `require`; it isn't shipped in the binary until the package pass.
- **Class:** deliberate removal.
- **Decision:** `downstream/libraries` and `downstream/require` re-blessed without the `dreamIni` entries; `dreamIni` left `[globals]`, `[declared]`, and `[modules]`.

## Logs

### 44. Under a domain with a log stream, `@jess/log` writes there, attributed

- **What:** a `ScriptDomain` given `DomainSpec::logs` collects every `@jess/log` record its contexts and workers write, as a `jess_runtime::LogRecord` carrying the runtime's attribution (domain, generation, context or worker); the runtime's own sink (`set_log_sink`) doesn't see them. A worker's records go there whether or not it shares any service. Without a stream, and outside any domain, `@jess/log` writes where it always did.
- **Class:** addition. No fixture changed: the fixtures run outside any domain.
- **Decision:** pinned in the kernel fixture (`a_domain_log_stream_collects_its_contexts_and_its_workers`).

## Bethesda archives

### 45. `dreamArchive`, `dreamArchivetool`, `@dream/archive`, and `@dream/archivetool` left the runtime

- **What:** jess no longer links dream_archive or dream_archivetool. The `dreamArchive` and `dreamArchivetool` globals, `require("@dream/archive")` and its `/ba2`, `/bsa`, `/bsa/tes3`, and `/bsa/tes4` submodules, `require("@dream/archivetool")`, the tool methods the second added to every archive handle (`toolInfo`, `verify`, `diff`, `extract`, `extractMany`, `planExtract`, `extractAll`, `planExtractAll`, and the `ByPathHex` forms), and the `Module__dream_archive` and `Module__dream_archivetool` types are gone from the plan, the compat globals, and `jess luau-types`. What replaces them is Dreamwind's `dream.bethesda-archives` layer, in Luau: `@dreamwind/archive` and its submodules, the archive API under its own name, and `@dreamwind/archive/tool`, the tool module's sixteen functions, which take an opened archive where the binding had a method on it, return plain tables where it returned report and plan userdata, and refuse `fsync = true`, since `@dream/vfs` can't flush. Both are held to the crates' answers by goldens recorded from the native bindings while jess still linked them, the ports agreeing on every case (`jpk/dreamwind/tests/archive/golden/differential.txt`, run by `libs/lua/tests/archive_experiment.rs`). vfstool still reads archives with dream_archive for the load-order VFS (its `beth-archives` feature, through jess-assets and jess-openmw); that link is vfstool's.
- **Class:** deliberate removal.
- **Decision:** `downstream/libraries` and `downstream/require` re-blessed without the archive entries; `dreamArchive` and `dreamArchivetool` left `[globals]`, `[declared]`, and `[modules]`.

### 46. `jess.nif.sceneBytes`, and `jess.nif.dumpBytes` takes `reader`

- **What:** `jess.nif.sceneBytes` returns jess-nif's scene of a Morrowind NIF as a table (`e8b3eab`), and `jess.nif.dumpBytes` takes `reader = 'gamebryo'` to read any version with the reader OpenMW uses (`cf48a1d`); both landed without their inventory entries.
- **Class:** addition.
- **Decision:** `sceneBytes` joins `[modules."jess.nif"]`, and `reader` joins `[options."jess.nif.dumpBytes"]`'s keys.

## Localization

### 47. `jess.l10n` left the native API for `@jess/l10n`, the built-in package's Luau

- **What:** the ICU MessageFormat registry is Luau now (`jpk/builtin/src/l10n`), over `@dream/intl`'s CLDR plural rules and decimal formatting. `jess.l10n` and the `L10nBundle` userdata are gone. Each runtime has its own registry, so the host no longer attaches one or posts `l10n.changed`. `l10n.read` and `l10n.configure` are gone too: a folder is read through `@dream/fs`, so `filesystem.read` gates it.
- **Class:** removal. `jess` lost the `l10n` member; `@jess/l10n` is still the require path, with the same functions plus `new`, `files`, `directory`, `addProvider`, `providers`, `formatPattern`, and `checkPattern`.
- **Decision:** the `@jess` member list in `contracts.rs` dropped `l10n`. The acceptance cases are `jpk/builtin/tests/l10n/run.jsl`, and the kernel fixture's script and worker each configure their own registry.

## Files

### 48. `jess.fs` left the native API for l3i's `@dream/fs`

- **What:** jess had a second file API next to l3i's, with its own errors (raising where `@dream/fs` answers `nil, message, kind`), its own list order (the filesystem's, where `@dream/fs` sorts), and its own backups. `jess.fs` and `src/lua/fs.rs` are gone; the built-in package, the tools, and the site use `@dream/fs`. 16 and 41 describe the removed API.
- **Class:** removal. `jess` lost the `fs` member, and the orphaned `fs/fs` fixture went with it.
- **Decision:** the member list in `contracts.rs` dropped `fs`. A backup before writing is a tool's policy, not a file API's.

## Regular expressions

### 49. `jess.regex` left the native API for l3i's `bytes.regex`

- **What:** jess and l3i each bound the `regex` crate, with different position conventions (jess's 1-based and inclusive, l3i's 0-based and exclusive) and different error text. `bytes.regex` gained what only `jess.regex` had (`match`, `captures`, `matches`, `replace`, `replaceAll`, `split`, `regexEscape`); `jess.regex`, `src/lua/regexp.rs`, the `Regex` types, and the `regex` dependency are gone.
- **Class:** removal. `jess` lost the `regex` member, and the `regex/regex` fixture went with it.
- **Decision:** the member list and the fixture list in `contracts.rs` dropped `regex`; `api/regex.md` maps every call, positions included.
