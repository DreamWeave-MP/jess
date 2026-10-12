--!strict
--!native
-- @jess/style's own cases: for every rule, code that follows it and code that doesn't, with the
-- findings the checker gives for the second, each as `line:column rule`.
local style = require '@jess/style'
local test = require '@jess/test'
local expect = test.expect
local suite = test.new()

type Case =
  { rule: string, good: string, bad: string, findings: { string }, options: style.Options? }

local Cases: { Case } = {
  {
    rule = 'syntax',
    good = 'local value = 1\n',
    bad = 'local value = \n',
    findings = { '2:1 syntax' },
  },

  {
    rule = 'pcall.comment',
    good = [[
local function load(path: string): string?
  -- `jess.fs.read` throws on a path the OS refuses; a missing file is nil.
  local loaded, text = pcall(jess.fs.read, path)
  if not loaded then return nil end

  return text
end

-- Each call throws on purpose; the messages are what's compared.
local _, first = pcall(error, 'one')
local _, second = pcall(error, 'two')

-- `decode` throws on bytes that don't parse.
local function decode(bytes: string): (boolean, unknown)
  return pcall(jess.luadata.decode, bytes)
end

print(load, first, second, decode)
]],
    bad = [[
local function load(path: string): string?
  local loaded, text = pcall(jess.fs.read, path)
  if not loaded then return nil end

  return text
end

-- A comment that isn't right above the call.
local value = 1
local ok = xpcall(load, print, 'x')
local passed = pcall
print(value, ok, passed)
]],
    findings = { '2:24 pcall.comment', '10:12 pcall.comment', '11:16 pcall.comment' },
    options = { only = { 'pcall.comment' } },
  },

  {
    rule = 'layout.indentation',
    good = 'if ready then\n  print(1)\nend\nlocal text = [[\n\tkept as written\n]]\nprint(text)\n',
    bad = 'if ready then\n\tprint(1)\n   print(2)\nend\n',
    findings = { '2:1 layout.indentation', '3:1 layout.indentation' },
  },

  {
    rule = 'layout.compound',
    good = [[
local first, second = 1, 2

if first then
  print(first)
end

for index = 1, second do
  if index == 1 then continue end

  print(index)
end

local lights = {
  'light_com_candle_01',
}

if second then print(second) end
if first then print(first) end
local count = #lights
print(count)
]],
    bad = [[
if first then
  print(first)
end
for index = 1, 2 do
  print(index)
end
if ready then return end
while waiting do
  wait()
end
local lights = {
  'light_com_candle_01',
}
-- A comment doesn't separate statements.
table.sort(lights, function(left: string, right: string): boolean
  return left < right
end)
]],
    findings = {
      '4:1 layout.compound',
      '7:1 layout.compound',
      '8:1 layout.compound',
      '11:1 layout.compound',
      '15:1 layout.compound',
    },
  },

  {
    rule = 'quotes.double',
    good = [[print('single', "it's", `{1}`)]],
    bad = [[print("double")]],
    findings = { '1:7 quotes.double' },
  },

  {
    rule = 'quotes.backtick',
    good = 'local count = 1\nprint(`{count} light`)\n',
    bad = 'print(`no holes`)\n',
    findings = { '1:7 quotes.backtick' },
  },

  {
    rule = 'hoisting',
    good = 'local Huge = math.huge\nprint(string.format(\'%d\', 1), Huge)\n',
    bad = 'local format, insert = string.format, table.insert\nlocal Type = type\nprint(format, insert, Type)\n',
    findings = { '1:7 hoisting', '1:15 hoisting', '2:7 hoisting' },
  },

  {
    rule = 'naming.case',
    good = [[
local Lights = { 'candle' }
local URL, H3UI = 'https://example.com', 'h3ui'
local count = 0

local function countLights(list: { string }): number
  local total = #list
  return total + count
end

print(Lights, URL, H3UI, countLights)
]],
    bad = [[
local Total = 0
Total += 1
local light_count = 2

local function Count(): number
  local Lights = 1
  return Lights + light_count
end

print(Count)
local IDENTITY = 1
print(IDENTITY)
]],
    findings = {
      '1:7 naming.case',
      '3:7 naming.case',
      '5:16 naming.case',
      '6:9 naming.case',
      '11:7 naming.case',
    },
  },

  {
    rule = 'naming.constant',
    good = [[
local MaxLights, EmptyOptions = 16, {}
local seen = {}
seen.candle = true
print(MaxLights, EmptyOptions, seen)
]],
    bad = [[
local maxLights = 16
local emptyOptions = {}
local names = { 'candle', 'torch' }
return { maxLights = maxLights, emptyOptions = emptyOptions, names = names }
]],
    findings = { '1:7 naming.constant', '2:7 naming.constant', '3:7 naming.constant' },
  },

  {
    rule = 'naming.words',
    good = [[
for key, value in { light = 1 } do
  print(key, value)
end

table.sort({ 2, 1 }, function(left: number, right: number): boolean return left < right end)

for index = 1, 2 do
  print { column = index, record = index, ['x'] = index }
end
]],
    bad = [[
for k, v in { light = 1 } do
  print(k, v)
end

table.sort({ 2, 1 }, function(a: number, b: number): boolean return a < b end)

for i = 1, 2 do
  print { x = i, rec = i }
end
]],
    findings = {
      '1:5 naming.words',
      '1:8 naming.words',
      '5:31 naming.words',
      '5:42 naming.words',
      '7:5 naming.words',
      '8:11 naming.words',
      '8:18 naming.words',
    },
  },

  {
    rule = 'truthiness',
    good = [[
local function describe(name: string?, flag: boolean?): string
  if not name then return 'nobody' end
  if flag == nil then return name end

  return `{name} {flag}`
end

local found = string.find('a', 'a') ~= nil
print(describe, found)
]],
    bad = [[
local function describe(name: string?): string
  if name == nil then return 'nobody' end
  if string.find(name, '^%s') ~= nil then return 'spaced' end

  return name
end

print(describe)
]],
    findings = { '2:6 truthiness', '3:6 truthiness' },
  },

  {
    rule = 'locals.group',
    good = 'local merged, failed = {}, {}\nlocal report\nlocal summary\nprint(merged, failed, report, summary)\n',
    bad = 'local merged = {}\nlocal failed = {}\nprint(merged, failed)\n',
    findings = { '2:1 locals.group' },
    options = { only = { 'locals.group' } },
  },

  -- Luau sizes an empty table for what is assigned into it only when its local is declared
  -- alone, so a table filled that way keeps a line to itself: by field, by method, by constant
  -- slot, or by the variable of a loop counting from 1 to a small constant.
  {
    rule = 'locals.group',
    good = [=[
local point = {}
local names = {}
local squares = {}
local handlers = {}
local seen, queue = {}, {}
point.x, point.y = 1, 2
names[1], names[2] = 'first', 'second'

for index = 1, 4 do
  squares[index] = index * index
end

function handlers.open() return point end

seen[names[1]] = true
table.insert(queue, squares)
print(handlers)
]=],
    bad = [=[
local point, names, squares, handlers, seen = {}, {}, {}, {}, {}
point.x = 1
names[1] = 'first'

for index = 1, 4 do
  squares[index] = index * index
end

function handlers.open() return point end

seen[names[1]] = true
]=],
    findings = {
      '1:7 locals.group',
      '1:14 locals.group',
      '1:21 locals.group',
      '1:30 locals.group',
    },
    options = { only = { 'locals.group' } },
  },

  -- Loops Luau reads no size from leave their table free to share a line.
  {
    rule = 'locals.group',
    good = [[
local stepped, long, offset = {}, {}, {}

for index = 1, 9, 2 do
  stepped[index] = index
end

for index = 1, 100 do
  long[index] = index
end

for index = 0, 4 do
  offset[index] = index
end
]],
    bad = 'local stepped = {}\nlocal long = {}\n\nfor index = 1, 100 do\n  long[index] = index\n  stepped[index * 2] = index\nend\n',
    findings = { '2:1 locals.group' },
    options = { only = { 'locals.group' } },
  },

  {
    rule = 'locals.mixed',
    good = 'local ok, problem = pcall(print)\nlocal report\nlocal count = 1\nprint(ok, problem, report, count)\n',
    bad = 'local count, report = 1\nprint(count, report)\n',
    findings = { '1:1 locals.mixed' },
    options = { only = { 'locals.mixed' } },
  },

  {
    rule = 'guard.else',
    good = [[
local function size(path: string): number
  if not jess.fs.exists(path) then return 0 end

  local bytes = jess.fs.size(path) or 0
  return bytes
end

print(size)
]],
    bad = [[
local function size(path: string): number
  if not jess.fs.exists(path) then
    return 0
  else
    local bytes = jess.fs.size(path) or 0
    return bytes
  end
end

print(size)
]],
    findings = { '4:7 guard.else' },
  },

  {
    rule = 'guard.nested',
    good = [[
for _, record in records do
  if not record.id then continue end

  local id = record.id
  table.insert(ids, id)
  print(id)
end
]],
    bad = [[
for _, record in records do
  if record.id then
    local id = record.id
    table.insert(ids, id)
    print(id)
  end
end
]],
    findings = { '2:3 guard.nested' },
  },

  {
    rule = 'directives.strict',
    good = '--!strict\n--!native\nprint(1)\n',
    bad = '--!native\nprint(1)\n',
    findings = { '1:1 directives.strict' },
    options = { directives = { 'strict', 'native' } },
  },

  {
    rule = 'directives.native',
    good = '--!strict\n--!native\nprint(1)\n',
    bad = '--!strict\nprint(1)\n',
    findings = { '1:1 directives.native' },
    options = { directives = { 'strict', 'native' } },
  },

  {
    rule = 'types.any',
    good = 'local values: { unknown } = {}\nlocal function first(list: { string }): string? return list[1] end\nprint(values, first)\n',
    bad = 'local values: { any } = {}\nlocal function first(list: { any }): any return list[1] end\nprint(values, first)\n',
    findings = { '1:17 types.any', '2:30 types.any', '2:38 types.any' },
    options = { noAny = true, only = { 'types.any' } },
  },
}

local function found(source: string, options: style.Options?): { string }
  local result = {}

  for _, finding in style.check(source, options) do
    table.insert(result, `{finding.line}:{finding.column} {finding.rule}`)
  end

  return result
end

-- Each case is checked for its own rule alone unless it says otherwise.
suite.describe('rules', function()
  for _, case in Cases do
    local options: style.Options = case.options or { only = { case.rule } }

    suite.it(
      `{case.rule}: code that follows it has no findings`,
      function() expect(found(case.good, options)).toEqual {} end
    )

    suite.it(
      `{case.rule}: code that breaks it has these findings`,
      function() expect(found(case.bad, options)).toEqual(case.findings) end
    )
  end

  suite.it('covers every rule', function()
    local covered = {}

    for _, case in Cases do
      covered[case.rule] = true
    end

    for rule in style.rules do
      expect(covered[rule] and rule).toBe(rule)
    end
  end)
end)

suite.describe('findings', function()
  suite.it('name what a single letter stands for', function()
    local finding = style.check('for i = 1, 2 do\n  print(i)\nend\n')[1]
    expect(finding.message).toBe '\'i\' is a single letter: name what it holds (index)'
  end)

  suite.it('carry a span, a severity, and a message', function()
    local finding = style.check('print("double")\n')[1]
    expect(finding.rule).toBe 'quotes.double'
    expect(finding.severity).toBe 'error'
    expect({ finding.line, finding.column, finding.endLine, finding.endColumn }).toEqual {
      1,
      7,
      1,
      14,
    }
    expect(style.format(finding, 'a.luau')).toBe(
      `a.luau:1:7: error[quotes.double]: {finding.message}`
    )
  end)

  suite.it('shout about a pcall without its comment', function()
    local finding = style.check('local ok = pcall(print)\n')[1]
    expect(finding.severity).toBe 'error'
    expect((string.find(finding.message, 'PCALL WITHOUT A COMMENT', 1, true))).toBe(1)
  end)

  suite.it('are limited to the rules asked for', function()
    expect(found('print("double")\nlocal Type = type\n', { only = { 'hoisting' } })).toEqual {
      '2:7 hoisting',
    }
    expect(function() style.check('', { only = { 'nothing' } }) end).toThrow 'there is no rule \'nothing\''
  end)
end)

suite.describe('profiles', function()
  suite.it('let Lua hoist globals into PascalCase locals, as s3stash does', function()
    local source =
      'local Next, Type = next, type\n\nlocal function count(list)\n  local Insert = table.insert\n  Insert(list, 1)\n  return Next(list), Type(list)\nend\n\nreturn count\n'
    expect(found(source, { profile = 'lua' })).toEqual {}
    expect(found(source)).toEqual {
      '1:7 hoisting',
      '1:13 hoisting',
      '4:9 hoisting',
      '4:9 naming.case',
    }
  end)

  suite.it('read LuaLS parameter types in Lua', function()
    local source =
      '---@param name? string\n---@param flag? boolean\nlocal function describe(name, flag)\n  if name == nil then return flag == nil end\n\n  return name\nend\n\nreturn describe\n'
    expect(found(source, { profile = 'lua' })).toEqual { '4:6 truthiness' }
  end)

  suite.it('treat a returned function as the module', function()
    local source =
      'return function(state: { [string]: unknown })\n  local Defaults = { fileName = \'out.esp\' }\n  print(state, Defaults)\nend\n'
    expect(found(source)).toEqual {}
  end)
end)

suite.describe('the l3i dialect', function()
  suite.it('is read as what was written, with nothing to report in code that follows the style', function()
    local source = table.concat({
      'local ids = [for entity in entities if entity.active => entity.id]',
      'local total = sum[for {id, position: {x: column}} in entities => id + column]',
      'local samples = buffer.create(64)',
      'local filled, written = into(samples, \'f32\')[for value in range(1, 16) => value * 0.5]',
      'local heavy = #[for value in samples[1:16, \'f32@4\'] if value > 4 => value]',
      'local {health, position: {x: across}}: Entity = entities[1]',
      'local function area({width, height}: Size): number return width * height end',
      'local pairs = [for index, name in enumerate(names) => `{index}:{name}`]',
      'local sums = [for left, right in zipStrict(lefts, rights) => left + right]',
      'print(ids, total, filled, written, heavy, health, across, area, pairs, sums, names[2:3])',
      '',
    }, '\n')

    expect(found(source)).toEqual {}
  end)

  suite.it('holds every binding to the naming rules', function()
    local source = table.concat({
      'local ids = [for e in entities => e.id]',
      'local {x, health: hp_left} = entity',
      'local function area({w, height: Tall}: Size): number return w * Tall end',
      'local firsts = [for {k}, i in zipShortest(records, indices) => k + i]',
      'print(ids, x, hp_left, area, firsts)',
      '',
    }, '\n')

    expect(found(source)).toEqual {
      '1:18 naming.words',
      '2:8 naming.words',
      '2:19 naming.case',
      '3:22 naming.words',
      '3:33 naming.case',
      '4:22 naming.words',
      '4:26 naming.words',
    }
  end)

  suite.it('reads the strings inside a comprehension, and an element kind, as strings', function()
    local source = table.concat({
      'local names = [for entity in entities => entity.name or "nobody"]',
      'local heights = terrain[1:count, "f32"]',
      'into(samples, "f32@16", 8)[for height in heights[1:count, \'f32\'] => height]',
      'print(names)',
      '',
    }, '\n')

    expect(found(source)).toEqual { '1:57 quotes.double', '2:34 quotes.double', '3:15 quotes.double' }
  end)

  suite.it('tests a filter, and what any and all reduce, for truth', function()
    local source = table.concat({
      'local function live(names: { string }, owner: string?): ({ string }, boolean)',
      '  return [for name in names if owner ~= nil => name], any[for name in names => owner ~= nil and name]',
      'end',
      '',
      'print(live)',
      '',
    }, '\n')

    expect(found(source)).toEqual { '2:32 truthiness', '2:80 truthiness' }
  end)

  suite.it('takes a table a sink fills for one that changes, and a buffer sink for two values', function()
    local source = table.concat({
      'local scratch = {}',
      '',
      'local samples = buffer.create(64)',
      '',
      'local function fill(values: { number }): number',
      '  local filled, written = into(samples, \'f32\')[for value in values => value]',
      '  into(scratch)[for value in values => value * 2]',
      '  return written + buffer.len(filled)',
      'end',
      '',
      'print(fill)',
      '',
    }, '\n')

    expect(found(source)).toEqual {}
  end)

  suite.it('gives a comprehension over several lines the room a table over several lines gets', function()
    local source = table.concat({
      'local ids = [',
      '  for entity in entities',
      '  if entity.active',
      '  => entity.id',
      ']',
      'if ids[1] then print(ids[1]) end',
      '',
    }, '\n')

    expect(found(source)).toEqual { '6:1 layout.compound' }
  end)

  suite.it('reports a comprehension with no projection as a syntax error', function()
    expect(found('local ids = [for entity in entities]\n')[1]).toBe '1:36 syntax'
  end)
end)

suite.describe('definitions', function()
  suite.it('parse as definitions when nothing else parses', function()
    local source = 'declare extern type Handle with\n  name: string\nend\n'
    expect(found(source)).toEqual {}
    expect(found(source, { declarations = false })[1]).toBe '1:1 syntax'
  end)
end)

suite.describe('a source Luau gives up on', function()
  suite.it('is reported by its syntax errors, with nothing in its tree to walk', function()
    -- The parser stops at its hundredth error and hands back a root that is one StatError.
    local findings = found(string.rep(') ', 100) .. '\n')
    expect(#findings > 0).toBeTruthy()

    for _, finding in findings do
      expect(string.match(finding, ' (%S+)$')).toBe 'syntax'
    end
  end)
end)

suite.describe('markdown', function()
  suite.it('finds every luau block and the line it starts on', function()
    local page = 'Text.\n\n```luau\nprint(1)\n```\n\n```sh\nls\n```\n\n  ```luau\nprint(2)\n  ```\n'
    local blocks = style.markdownBlocks(page)
    expect(#blocks).toBe(2)
    expect({ blocks[1].line, blocks[1].source }).toEqual { 3, 'print(1)\n' }
    expect({ blocks[2].line, blocks[2].source }).toEqual { 11, 'print(2)\n' }
  end)
end)

return suite
