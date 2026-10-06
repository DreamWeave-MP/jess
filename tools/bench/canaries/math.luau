--!strict
--!native
-- `math.*` in a tight loop: the fastcalls a safe environment and native code give builtins.
local _, scaleText = ...
local count = 5000000 * (tonumber(scaleText) or 1)

local started, total = os.clock(), 0
for index = 1, count do
  local root = math.sqrt(index)
  total += math.floor(root * 1.5) + math.abs(math.sin(index)) + math.clamp(index % 17, 2, 9)
  total += math.max(index % 3, 1) + math.min(root, 100)
end

assert(total > 0)
print(`canary {os.clock() - started}`)
