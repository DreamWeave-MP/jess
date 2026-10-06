--!strict
-- Reading fields through record views, the way the bundled scripts compare records.
local folder = ...
local Rounds = 4

local paths = {}
for index = 1, 8 do
  table.insert(paths, jess.fs.join(folder, `lights-{index}.esp`))
end

local lights: { LightRecord } = {}
for _, plugin in jess.plugin.openAll(paths, { types = { 'LIGH' }, decode = true, openmw = false }) do
  for _, record in plugin:select({ type = 'LIGH' }):list() do
    table.insert(lights, record :: LightRecord)
  end
end

local started, total = os.clock(), 0
for _ = 1, Rounds do
  for _, light in lights do
    local data = light.data
    total += data.radius + data.time + data.color[1] + #light.name
  end
end

assert(total > 0)
print(`canary {os.clock() - started}`)
