--!strict
-- `jess.plugin.openAll` keeping one record type: the parallel, filtered parse every bundled script
-- starts with.
local folder = ...
local Rounds = 20

local paths = {}
for index = 1, 8 do
  table.insert(paths, jess.fs.join(folder, `lights-{index}.esp`))
end

local started, count = os.clock(), 0
for _ = 1, Rounds do
  local plugins = jess.plugin.openAll(paths, { types = { 'LIGH' }, decode = true, openmw = false })
  count += #plugins
end

assert(count == Rounds * #paths)
print(`canary {os.clock() - started}`)
