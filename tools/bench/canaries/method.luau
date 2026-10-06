--!strict
-- A native method in a hot loop: `plugin:find` by id and `jess.meta` on what it finds, the lookups
-- a merge makes per record.
local folder = ...
local Rounds = 100
local plugin = jess.plugin.open(jess.fs.join(folder, 'lights-1.esp'), { openmw = false })

local ids = {}
for record in plugin:select { type = 'LIGH' } do
  table.insert(ids, jess.meta(record).id :: string)
end

local started, found = os.clock(), 0
for _ = 1, Rounds do
  for _, id in ids do
    local record = plugin:find('LIGH', id)
    if record and jess.meta(record).tag == 'LIGH' then
      found += 1
    end
  end
end

assert(found == Rounds * #ids)
print(`canary {os.clock() - started}`)
