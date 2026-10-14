--!strict
--[[
Fails unless builtins run on Luau's fast path.

Luau resolves `math.floor` and `buffer.readi32` once and calls them directly only in an environment
marked safe. Jess marks its globals safe; regressions here have caused order-of-magnitude slowdowns
for large Luau workloads.
This runs one loop twice: as a closure in the script's own environment, and as a second closure of
the same function moved into a copy of that environment, which `setfenv` marks unsafe. It fails when
the first isn't several times faster. The ratio doesn't depend on the machine, so the test suite
runs this too.

The floor is 3x. On l3i's Luau build the unsafe copy is itself fast, so a safe environment measures
4x to 5x faster (mlua's measured over 5x); one that isn't marked safe measures about 1x, which is
what this exists to catch.
]]
local _, scaleText = ...
local count = 200000 * (tonumber(scaleText) or 1)
local MinimumRatio = 3

local function makeLoop(): (number) -> number
  return function(count: number): number
    local data, total = buffer.create(4096), 0
    for index = 1, count do
      local offset = (index % 1024) * 4
      buffer.writei32(data, offset, math.floor(math.sqrt(index) * 1.5))
      total += buffer.readi32(data, offset) + math.abs(-index) % 7 + math.max(index % 3, 1)
    end

    return total
  end
end

local safe, unsafe = makeLoop(), makeLoop()
setfenv(unsafe, table.clone(_G))

-- The best of three of each, alternating, so a busy machine slows both alike.
local safeTime, unsafeTime = math.huge, math.huge
for _ = 1, 3 do
  local started = os.clock()
  local expected = safe(count)
  safeTime = math.min(safeTime, os.clock() - started)

  started = os.clock()
  assert(unsafe(count) == expected, 'the unsafe copy computed something else')
  unsafeTime = math.min(unsafeTime, os.clock() - started)
end

local ratio = unsafeTime / safeTime
print(`safe {safeTime} s, unsafe {unsafeTime} s, {ratio}x`)
assert(
  ratio >= MinimumRatio,
  `builtins aren't on the fast path: the safe environment is only {ratio}x faster than an unsafe one (want {MinimumRatio}x); is the global table still marked safe?`
)
print(`canary {safeTime}`)
