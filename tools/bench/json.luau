--!strict
--!native
--[[
JSON for the benchmark harness's results files: tables out as text, keys sorted, with the
indentation given (none for one line). `@jess/json` writes one line in its own value model; a
results file is read by people and diffed, so it gets the indented form Python's json.dumps gave
the baselines. Every byte outside printable ASCII is escaped, so a record id in Windows-1252 comes
out as the Latin-1 code points the old harness wrote.

  local json = require './json'
  json.encode(value, 1)
]]

local Escape = {
  ['"'] = '\\"',
  ['\\'] = '\\\\',
  ['\b'] = '\\b',
  ['\f'] = '\\f',
  ['\n'] = '\\n',
  ['\r'] = '\\r',
  ['\t'] = '\\t',
}

local function fail(message: string): never error(`json: {message}`, 0) end

local function jsonString(value: string): string
  local escaped = string.gsub(value, '[%z\1-\31\\"\127-\255]', function(character: string): string
    local known = Escape[character]
    if known then return known end

    return string.format('\\u%04x', string.byte(character))
  end)
  return `"{escaped}"`
end

local function arrayLength(value: { [unknown]: unknown }): number?
  local count, highest = 0, 0
  for key in value do
    if type(key) ~= 'number' or key < 1 or key % 1 ~= 0 then return nil end

    count += 1
    highest = math.max(highest, key)
  end

  if count ~= highest then return nil end

  return highest
end

local encode: (value: unknown, indent: number?, depth: number) -> string

local function encodeTable(value: { [unknown]: unknown }, indent: number?, depth: number): string
  local open, close, separator = '', '', ','
  if indent then
    open = '\n' .. string.rep(' ', indent * (depth + 1))
    close = '\n' .. string.rep(' ', indent * depth)
    separator = ',' .. open
  end

  local length = arrayLength(value)
  if length then
    if length == 0 then return '[]' end

    local parts = table.create(length)
    for index = 1, length do
      parts[index] = encode(value[index], indent, depth + 1)
    end

    return `[{open}{table.concat(parts, separator)}{close}]`
  end

  local keys: { string } = {}
  for key in value do
    if type(key) ~= 'string' then fail('result tables need string keys or dense array indexes') end

    table.insert(keys, key)
  end

  if #keys == 0 then return '{}' end

  table.sort(keys)
  local colon = if indent then ': ' else ':'
  local parts = table.create(#keys)
  for index, key in keys do
    parts[index] = `{jsonString(key)}{colon}{encode(value[key], indent, depth + 1)}`
  end

  return `\{{open}{table.concat(parts, separator)}{close}}`
end

encode = function(value: unknown, indent: number?, depth: number): string
  local kind = type(value)
  if kind == 'string' then return jsonString(value :: string) end

  if kind == 'number' then
    local number = value :: number
    if number ~= number or number == math.huge or number == -math.huge then
      fail('cannot write a non-finite number')
    end

    return tostring(number)
  end

  if kind == 'boolean' then return tostring(value) end
  if kind == 'table' then return encodeTable(value :: { [unknown]: unknown }, indent, depth) end
  if value == nil then return 'null' end

  fail(`cannot encode a value of type {kind}`)
end

--- `value` as JSON: on one line without `indent`, else over lines indented by that many spaces.
--- Tables with dense integer keys are arrays; all others need string keys. `nil` inside an array
--- can't be represented; a field that is nil is simply absent.
local function encodeValue(value: unknown, indent: number?): string return encode(value, indent, 0) end

return table.freeze { encode = encodeValue, string = jsonString }
