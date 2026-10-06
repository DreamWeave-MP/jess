--!strict
--!native
-- The built-in text decoders' contract cases. The larger differential/fuzz corpus lives outside
-- this package test: these are the small cases a Jess build should never regress.
local json = require '@jess/json'
local toml = require '@jess/data/toml'
local yaml = require '@jess/yaml'
local test = require '@jess/test'
local expect = test.expect
local suite = test.new()

suite.describe('json', function()
  suite.it('decodes RFC 8259 values and UTF-16 escapes', function()
    local value = json.decode '{"text":"\\uD83D\\uDE03","values":[1,-2.5e2,true,false,null]}'
    expect(value.text).toBe '😃'
    expect(value.values[1]).toBe(1)
    expect(value.values[2]).toBe(-250)
    expect(value.values[3]).toBe(true)
    expect(value.values[4]).toBe(false)
    expect(value.values[5]).toBe(json.null)
  end)

  suite.it('is strict about separators, numbers, strings, and trailing data', function()
    local invalid = {
      '[1,]',
      '{"a":1,}',
      '01',
      '+1',
      '1.',
      '1e',
      '"\\q"',
      '"\\uD800"',
      'true false',
    }

    for _, source in invalid do
      expect(function() json.decode(source) end).toThrow 'json:'
    end
  end)
end)

suite.describe('toml', function()
  suite.it('implements TOML 1.1 values and keeps the native layout side channel', function()
    local value, layout = toml.decode [=[
name = "Jess"
max = 9223372036854775807
small = 42
float = 2.0
when = 1979-05-27T07:32Z
mixed = [1, "two", true,]
inline = {
  one = 1,
  two = 2,
}
]=]
    expect(value.name).toBe 'Jess'
    expect(typeof(value.small)).toBe 'number'
    expect(value.small).toBe(42)
    expect(typeof(value.max)).toBe 'integer'
    expect(tostring(value.max)).toBe '9223372036854775807'
    expect(value.when).toBe '1979-05-27T07:32Z'
    expect(value.mixed).toEqual { 1, 'two', true }
    expect(value.inline).toEqual { one = 1, two = 2 }
    expect(layout[value].keys).toEqual { 'name', 'max', 'small', 'float', 'when', 'mixed', 'inline' }
    expect(layout[value].floats.float).toBe(true)
  end)

  suite.it('implements dotted tables and arrays of tables', function()
    local value = toml.decode [=[
[fruit.apple]
smooth = true
[fruit]
name = "apple"
[[products]]
name = "Hammer"
[[products]]
name = "Nail"
[products.detail]
color = "gray"
]=]
    expect(value.fruit.apple.smooth).toBe(true)
    expect(value.fruit.name).toBe 'apple'
    expect(value.products[1].name).toBe 'Hammer'
    expect(value.products[2].detail.color).toBe 'gray'
  end)

  suite.it('rejects TOML structural and numeric errors', function()
    local invalid = {
      'a = 01',
      'a = 1_',
      'a = 0x8000000000000000',
      'a = 1979-02-29',
      'a = "\\q"',
      'a = 1\na = 2',
      'a.b = 1\n[a]',
      'a = { b = 1 }\na.c = 2',
    }

    for _, source in invalid do
      expect(function() toml.decode(source) end).toThrow 'toml:'
    end
  end)
end)

suite.describe('yaml', function()
  suite.it('uses the YAML 1.2 Core schema', function()
    local value = yaml.decode [[
nulls: [null, Null, NULL, ~]
booleans: [true, True, FALSE]
integers: [12, -19, 0o17, 0x3A]
floats: [.5, +12e03, -.Inf, .NAN]
strings: [yes, no, on, off]
]]
    for _, item in value.nulls do expect(item).toBe(yaml.null) end
    expect(value.booleans).toEqual { true, true, false }
    expect(value.integers).toEqual { 12, -19, 15, 58 }
    expect(value.floats[1]).toBe(0.5)
    expect(value.floats[2]).toBe(12000)
    expect(value.floats[3]).toBe(-math.huge)
    expect(value.floats[4] ~= value.floats[4]).toBe(true)
    expect(value.strings).toEqual { 'yes', 'no', 'on', 'off' }
  end)

  suite.it('loads block, flow, anchor, alias, and custom-tag nodes', function()
    local value = yaml.decode [[
first: &base
  name: Jess
  folded: >-
    one
    two
  flow: [1, {two: 2},]
  literal: |
    a
    b
second: *base
custom: !<tag:example.com,2026:thing> hello
]]
    expect(value.second == value.first).toBe(true)
    expect(value.first.folded).toBe 'one two'
    expect(value.first.literal).toBe 'a\nb\n'
    expect(value.first.flow[2].two).toBe(2)
    expect(value.custom.tag).toBe 'tag:example.com,2026:thing'
    expect(value.custom.value).toBe 'hello'
  end)

  suite.it('loads independent documents and rejects malformed graphs', function()
    local documents = yaml.decodeAll '---\none\n...\n---\ntwo\n...\n'
    expect(documents).toEqual { 'one', 'two' }
    expect(function() yaml.decode '[1,,2]' end).toThrow 'YAML:'
    expect(function() yaml.decode 'value: *missing' end).toThrow 'YAML:'
    expect(function() yaml.decode 'a: 1\na: 2' end).toThrow 'YAML:'
  end)
end)

return suite
