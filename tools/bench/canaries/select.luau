--!strict
-- Selecting and iterating records: a selector over every plugin, walked handle by handle.
local folder = ...
local Rounds = 40

local paths = {}
for index = 1, 8 do
  table.insert(paths, jess.fs.join(folder, `lights-{index}.esp`))
end

local plugins = jess.plugin.openAll(paths, { openmw = false })

local started, count = os.clock(), 0
for _ = 1, Rounds do
  for _, plugin in plugins do
    for _ in plugin:select { type = 'LIGH' } do
      count += 1
    end
  end
end

assert(count > 0)
print(`canary {os.clock() - started}`)
