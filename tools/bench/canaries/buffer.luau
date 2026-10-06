--!strict
--!native
-- `buffer.*` reads and writes in a tight loop, the shape of Merged Lands' vertex walks.
local _, scaleText = ...
local count = 4000000 * (tonumber(scaleText) or 1)
local data = buffer.create(65 * 65 * 4)

local started, total = os.clock(), 0
for index = 0, count - 1 do
  local offset = (index % 4225) * 4
  buffer.writei32(data, offset, buffer.readi32(data, offset) + index % 7)
  buffer.writeu8(data, offset, index % 256)
  total += buffer.readi32(data, offset) + buffer.readu16(data, offset)
  buffer.writef32(data, offset, total % 1000)
  total += buffer.readf32(data, offset)
end

assert(total > 0)
print(`canary {os.clock() - started}`)
