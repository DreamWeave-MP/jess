# Runtime decisions

The contract fixtures pin what a script observes of the runtime hosting jess. Three run today, from `tests/lua/contracts.rs` (`generic_contract_fixtures_keep_their_answers`): `input/acceptance`, a headless input runtime driven from Luau through `@jess/input`, and `shader/program` and `shader/errors`, a shader module's values and every mistake it can report. Each fixture is a `.jsl` script handed the `oracle.jsl` table, which records what the script sees (`raises`, `message`, `fnv`, `keys`, `sorted`, `collect`) and encodes it as JSON (`finish`); the result is compared with the `.expected.json` beside it. Re-blessing a fixture means rewriting that file on purpose, with the reason in the commit.

The entries below are the decisions behind what the fixtures and the neighbouring tests pin: what the behaviour is, whether it is compatibility (scripts may rely on it; keep it), a deliberate break, or a design choice, and how the tests encode it. A fixture never pins a bug. Where behaviour is dubious but relied on, the entry says so, so changing it is a deliberate act.

## Values and errors

### 1. The libraries hand out views, not tables

- **What:** a list a native module returns is a sequence or stream view on l3i: `#`, `[i]`, `for _, x in view`, and `:toTable()` work; `ipairs`, `pairs`, and `table.clone` over it fail. Indices are 1-based, sizes and hashes are Luau integers, option tables are strict, and a native module's errors read `dream.<module>: <context>: <message>`. Each module's own docs and the plan's type definitions are the contract.
- **Class:** deliberate, made upstream in l3i.
- **Decision:** pinned as views: a fixture that needs a table for its JSON copies the view with `:toTable()`, so the view is what the script exercises.

### 2. `collectgarbage` and `loadstring` are gone

- **What:** l3i opens Luau's standard libraries without `collectgarbage` (the host drives the collector; `jess.bench.collect` asks for a collection on purpose) and without `loadstring`.
- **Class:** deliberate, made upstream.
- **Decision:** nothing jess ships calls either; `tests/lua/contracts.rs` (`bench_names_the_build_and_collects`) pins the one collection a script can ask for.

### 3. Errors are messages

- **What:** an error jess raises reaches Luau as a plain string, so `pcall`, `xpcall`, `coroutine.resume`, a callback into Luau, and the host's report all see the message alone, never a traceback inside the value. A traceback is printed only for an error nothing caught, by the runner.
- **Class:** design.
- **Decision:** the fixtures assert messages, through `oracle.raises` and `oracle.message`.

### 4. `jess.null` is a null light userdata

- **What:** `type(jess.null) == "userdata"`, equal to itself, unequal to everything else, and the one null every codec reads and writes: `require('@jess/json').null`, `@jess/yaml`'s, and `@jess/data/value`'s `Null` are the same value.
- **Class:** compatibility.
- **Decision:** kept; `tests/lua/stdlib.rs` and the built-in package's data suite pin that the codecs share it.

### 5. Option tables and trailing arguments may be left out

- **What:** l3i's binder is fixed-arity and validates argument counts before conversions. Every option table and every argument a script may omit is declared optional in the plan, and the messages for the arguments jess checks itself (`expected a buffer or string, not table`) are jess's, carried inside l3i's `bad argument` wording. An option key a function doesn't take is an error naming the key and the ones it does take.
- **Class:** compatibility.
- **Decision:** pinned where the fixtures pass options; the unknown-key message is what `jess.worker.spawn`'s and the data codecs' tests assert.

### 6. serde's error wording is kept

- **What:** values jess deserializes natively fail with serde's standard spellings (`missing field \`weight\``), not a rewording.
- **Class:** compatibility.
- **Decision:** jess deserializes through its own error type, whose `custom` is all serde needs to spell every error the standard way (`convert::DeserializeError`).

### 7. `typeof` names userdata by their l3i names

- **What:** `typeof` of a native handle is its declared type name under `jess.`; `type()` still says `userdata`.
- **Class:** implementation detail.
- **Decision:** not pinned.

## Loading and native code

### 8. The safe-environment canary's floor is 3x

- **What:** `tools/bench/canaries/safeenv.jsl` compares a loop in the script's environment with one moved into an unsafe copy. Luau resolves library functions once and calls them directly only in an environment marked safe, so the safe one has to stay substantially faster; on l3i's build it measures 4x to 5x, and an environment not marked safe about 1x.
- **Class:** measurement, not behaviour.
- **Decision:** the floor is 3 (`MinimumRatio`), which still fails the moment the globals aren't marked safe.

### 9. Scripts load on the thread that runs them

- **What:** jess compiles each chunk once per runtime, by chunk name, and loads the bytecode on the running thread through `LoadScope::load_bytecode`, so Luau pre-resolves builtin imports against the globals the script will see, and a module reached through `require` is compiled under the same native-code policy as the script that required it.
- **Class:** implementation detail.
- **Decision:** none visible to scripts.

### 10. Native code is generated only for the chunks that ask for it

- **What:** the plan runs `NativeCodeMode::Annotated` (`src/lua/plan.rs`): a chunk marked `--!native` (or a function marked `@native`) compiles to native code, and no other does. Generating it for every chunk cost a tool run a third of a second before its first record, most of it in modules the run never called. Which modules earn the mark is a package's bench's business.
- **Class:** measurement; a deliberate policy.
- **Decision:** `Annotated`, recorded here and in the plan's builder; `jess run --profile` reports native blocks executed, so a chunk that should have been marked shows up as zero.

## Shaders

### 11. A number in a shader is a `Float`

- **What:** a Luau number has no `Float`/`Int`/`UInt` type, and `@jess/shader` takes one wherever a shader value goes (`shader.max(facing, 0)`, `2 * color`, `shader.vec4(position, 1)`). It is a `Float` literal, written at the operation's line. A number is never an `Int` or `UInt` by its value: integers are spelled `shader.int(3)` and `shader.uint(3)`. Inside an integer vector's constructor (`shader.vec3i(1, -2, 3)`) a number is that vector's integer, by where it is written, and must be whole and in range. A Luau boolean is a `Bool` constant.
- **Class:** design.
- **Decision:** pinned in `shader/program` (`constant`, `float`, `integers`); a number that isn't whole or is out of range is an error naming the constructor (`shader/errors`, `notWhole`, `uintRange`), a bare number beside an integer vector stays a `Float` (`integerFloat`), and a boolean is a `Bool` (`shader/program`, `logic`).

### 12. A texture comes with its sampler

- **What:** the IR needs an explicit `Sampler`. `material = { diffuse = shader.texture2d }` declares a `Texture2D` and a `Sampler` both named `diffuse` (they lower to `t_material_diffuse` and `s_material_diffuse`), and `sample(texture, uv)` pairs them. A sampler declared on its own (`material = { point = shader.sampler }`) goes between: `sample(texture, sampler, uv)`. The two forms differ by argument count, never by probing what an argument is. A texture and a sampler are handles, not values: arithmetic on one fails naming what it is.
- **Class:** design.
- **Decision:** both pinned in `shader/program` (`texture`, `sampler`, `explicitSampler`); `sample` of anything but a texture, or with anything but a sampler, is an error naming what it got (`shader/errors`, `notATexture`, `notASampler`, `textureAsValue`).

### 13. Shader errors name the line, never a column

- **What:** Luau's debug info records lines. A shader value's origin is the chunk and line of the nearest Luau function on the call stack (so a call through `pcall` still points at Luau source), and the module that called `shader.program` is its owner: `shaders/lit.jsl:12 (shaders/lit.jsl): dot(Vec3f, Vec4f) is invalid: expected two vectors of the same type`. A mistake only the whole program shows (a `Vec3f` position, a texture sampled in the vertex stage) is reported when the program is finished, every problem on its own line under `<owner>: the shader program is invalid`.
- **Class:** design.
- **Decision:** pinned whole in `shader/errors` (`operations`, `program`), line numbers included: moving a line in that fixture is a re-bless.

### 14. `shader.program` validates and doesn't fold

- **What:** the program a module returns is validated and otherwise as written. Constant folding (`jess::shader::Program::fold_constants`) is what the consumer does before lowering: it rewrites the program, and rewriting by default is what jess doesn't do.
- **Class:** design.
- **Decision:** nothing a script observes; `tests/shader` folds before it lowers.

### 15. A program declares its attributes and reads them by namespace

- **What:** a vertex shader reads what its mesh carries, and jess can't know what that is. `attributes` and `instance` declare the per-vertex and per-instance values by name and type, and the vertex function reads them as `input.attributes.position` and `input.instance.transform`, so they never collide with a parameter, a semantic, or a varying. Every declared attribute is in the vertex layout whether or not the program reads it: the layout is what a mesh is written in, and reading must not change it. A read records the line it was written on, not the declaration's.
- **Class:** design.
- **Decision:** pinned in `shader/program` (`attributes`, `instance`, `placed`, `shade`, `transform`) and `shader/errors` (`attributesInFragment`, `unknownAttribute`, `noInstanceAttributes`, `attributeType`, `namespaceName`, and the line `position` reports).

### 16. A program declares what each frequency binds, and nothing is built in

- **What:** a program's uniform values, textures, and samplers are declared under the frequency that binds them: `pass`, `material`, or `draw`, one bind group each. There are no semantic inputs: a camera, a model matrix, or a time is a member some frequency declares, often from a shared module of `shader.struct` types. Members are read by name, a struct's fields by name, an array's elements by a number or a shader value from 0, and `#` is an array's length; a step that doesn't reach a value is a `ShaderPlace`, which isn't one. A number index is checked against the array's length where it's written; a value index can't be, and isn't. What a program doesn't declare is only what the pipeline itself gives a stage, under `input.builtin`: `vertexIndex` and `instanceIndex` in the vertex function, `position` and `frontFacing` in the fragment function, each refused in the other.
- **Class:** design.
- **Decision:** pinned in `shader/program` (`places`, `builtins`, `lit`'s `pass`, `material`, `draw`, and the struct and array type names) and `shader/errors` (`places`, `types`, `structAsValue`, `uniformStride`, `builtinName`, and the spec's `unknownKey`, which names the keys a program takes).

## Modules

### 17. jess's members are `@jess/<name>`

- **What:** `require("@jess/shader")` is `jess.shader`, because a shader module starts with that line, and so is every other member table (`@jess/context` is `jess.context`, `@jess/worker` is `jess.worker`). `require` is typed to return the member's type for these names and `any` for a path.
- **Class:** addition.
- **Decision:** pinned in `shader/program` (`sameModule`) and in `tests/lua/contracts.rs` (`only_generic_members_are_installed`), which checks each member against its `require`.

### 18. `@jess/log`, `@jess/test`, `@jess/jpk`, and `@jess/package` are built-in modules written in Luau

- **What:** they are modules jess ships, embedded in the binary, installed through the runtime plan like the native `@dream/*` modules (`src/lua/stdlib.rs`), frozen, and typed in the definitions. `@jess/` is jess's namespace: a `@jess/` name that isn't a member, a built-in module, or an export of the built-in package is `module not found`, never a file lookup, so no file can stand in for one. `@jess/log` writes through one native sink the host selects (`set_log_sink`); scripts can't reach it.
- **Class:** addition.
- **Decision:** `tests/lua/stdlib.rs` pins freezing, the private sink, the closed namespace, and that neither a file beside a script nor a package's own module shadows them.

## Domains

### 19. `jess.input` binds to the domain's service, and checks capabilities only there

- **What:** under a `ScriptDomain` that carries an input service (the host attaches one), `jess.input.runtime()` is the application's live input: the host ticks it, and `tick` and `advance` are refused. Anywhere else each runtime keeps a headless runtime of its own, which is what the input fixture drives. Every runtime method checks its capability (`input.actions`, `input.observe`, `input.configure`) through the broker when the script runs under a domain; outside any domain the grant is the runtime's, what the command line or a target's declaration gave it.
- **Class:** addition. The fixture runs outside any domain, with the host-script grant.
- **Decision:** pinned in the kernel tests (`tests/lua/kernel.rs`: `the_application_kernel_end_to_end`, `granted_asks_the_broker_under_a_domain`) and in `tests/lua/lua_api.rs`; the denial is the broker's, `capability 'input.actions' denied to domain#1 gen#1 context#2`, as `pcall` reads it.

### 20. A worker shares the domain's services only when its spawner says `shareServices = true`

- **What:** `jess.worker.spawn` takes `shareServices`, a boolean: `true` gives the worker every service its domain carries, so its `jess.input.runtime()` is the application's; absent or `false`, the worker has none and its runtime is its own. A script can't name a Rust type, so the choice is all or nothing; the worker's capabilities still say what it may do with what it sees.
- **Class:** addition. The unknown-option message names the option (`expected capabilities, name, shareServices, or source`).
- **Decision:** pinned in the kernel tests (`the_application_kernel_end_to_end`).

### 21. Under a domain with a log stream, `@jess/log` writes there, attributed

- **What:** a `ScriptDomain` given `DomainSpec::logs` collects every `@jess/log` record its contexts and workers write, as a `jess::runtime::LogRecord` carrying the runtime's attribution (domain, generation, context or worker); the runtime's own sink doesn't see them. A worker's records go there whether or not it shares any service. Without a stream, and outside any domain, `@jess/log` writes where it always did.
- **Class:** addition.
- **Decision:** pinned in the kernel tests (`a_domain_log_stream_collects_its_contexts_and_its_workers`).

## What moved to Luau

Four native APIs left `jess` for Luau, each pinned by the built-in package's own suite (`jpk/builtin/tests`) rather than by a fixture here:

- **Localization** is `@jess/l10n`, the built-in package's Luau over `@dream/intl`. Each runtime has its own registry; reading a folder of bundles goes through `@dream/fs`, so `filesystem.read` gates it.
- **Files** are l3i's `@dream/fs`, which answers `nil, message, kind` where jess's old API raised, lists in sorted order, and keeps no backups; a backup before writing is `@jess/backup`'s, a tool's policy, not a file API's.
- **Regular expressions** are l3i's `bytes.regex`, with 0-based, end-exclusive positions.
- **Input policy** is `@jess/input`: the binding builders, labels, presentation, conflicts, and intent routing are Luau over the document and the action events. `jess.input` keeps what is native: `graph`, `control`, `formatVersion`, and `runtime`. The binding document is camelCase, binding locations count from 1, and `tests/lua/contracts.rs` checks that `@jess/input` wraps `jess.input` rather than being it.
