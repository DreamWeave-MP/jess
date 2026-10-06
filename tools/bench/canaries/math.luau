--!strict
--!native
-- `math.*` in a tight loop: the fastcalls a safe environment and native code give builtins.
local function run(scale: number): number
  local count = math.max(1, math.floor(5000000 * scale))

  local started, total = os.clock(), 0
  for index = 1, count do
    local root = math.sqrt(index)
    total += math.floor(root * 1.5) + math.abs(math.sin(index)) + math.clamp(index % 17, 2, 9)
    total += math.max(index % 3, 1) + math.min(root, 100)
  end

  assert(total > 0)
  return os.clock() - started
end

return table.freeze {
  name = 'math',
  path = 'tools/bench/canaries/math.luau',
  run = run,
}
