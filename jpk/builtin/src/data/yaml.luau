--!strict
--!native
-- YAML 1.2.2 loader using the Core schema. The parser is deliberately self-contained: YAML is a
-- built-in Jess data format, not a privileged Rust path. Presentation trivia is discarded, while
-- anchors/aliases preserve graph identity and custom tags are represented explicitly.

local Common = require './common'
local Values = require './value'

export type Options = {
  maxDepth: number?,
}

export type Tagged = {
  tag: string,
  value: any,
}

export type Value = any

local Null = Common.null
local Huge = math.huge
local SafeIntegerMax = integer.fromstring('9007199254740991')
local SafeIntegerMin = integer.fromstring('-9007199254740991')

local source = ''
local length = 0
local maxDepth = 512

local Lines: { any } = {}
local Anchors: { [string]: any } = {}
local TagHandles: { [string]: string } = {}

local function failAt(at: number, message: string): never
  Common.fail('YAML', source, at, message)
  error('unreachable', 0)
end

local function failLine(index: number, column: number, message: string): never
  local line = Lines[index]
  failAt((line and line.start or length + 1) + column - 1, message)
  error('unreachable', 0)
end

local function trimRight(text: string): string
  return (string.gsub(text, '[ \t]+$', ''))
end

local function trim(text: string): string
  return (string.gsub(string.gsub(text, '^[ \t]+', ''), '[ \t]+$', ''))
end

local function isSeparation(byte: number?): boolean
  return byte == nil or byte == 32 or byte == 9 or byte == 10 or byte == 13
end

local function isFlowDelimiter(byte: number?): boolean
  return byte == nil or byte == 44 or byte == 91 or byte == 93 or byte == 123 or byte == 125
end

local function isNameEnd(byte: number?): boolean
  return isSeparation(byte) or byte == 44 or byte == 91 or byte == 93 or byte == 123 or byte == 125
end

local function normalize(text: string): string
  Common.validateUtf8('YAML', text)
  if string.sub(text, 1, 3) == '\239\187\191' then text = string.sub(text, 4) end
  text = string.gsub(text, '\r\n', '\n')
  text = string.gsub(text, '\r', '\n')
  return text
end

local function buildLines()
  table.clear(Lines)
  local lineStart = 1
  local index = 1

  while lineStart <= length + 1 do
    local newline = string.find(source, '\n', lineStart, true)
    local finish = newline and newline - 1 or length
    local text = string.sub(source, lineStart, finish)
    local indent = 0

    while string.byte(text, indent + 1) == 32 do indent += 1 end

    if string.byte(text, indent + 1) == 9 then
      failAt(lineStart + indent, 'tab character may not be used for indentation')
    end

    Lines[index] = {
      text = text,
      start = lineStart,
      finish = finish,
      indent = indent,
      ended = newline ~= nil,
    }
    index += 1

    if not newline then break end
    lineStart = newline + 1
    if lineStart == length + 1 then
      Lines[index] = {
        text = '',
        start = lineStart,
        finish = length,
        indent = 0,
        ended = false,
      }
      break
    end
  end
end

-- Strip a block-line comment. A '#' starts a comment only at the start of the content or after
-- separation white space; quoted and flow collection contents are skipped so their '#' bytes are
-- ordinary scalar data.
local function stripComment(text: string): string
  local quote: number? = nil
  local depth = 0
  local index = 1
  local size = #text

  while index <= size do
    local byte = string.byte(text, index)
    if quote == 39 then
      if byte == 39 then
        if string.byte(text, index + 1) == 39 then
          index += 2
          continue
        end
        quote = nil
      end
    elseif quote == 34 then
      if byte == 92 then
        index += 2
        continue
      elseif byte == 34 then
        quote = nil
      end
    else
      if byte == 39 or byte == 34 then
        quote = byte
      elseif byte == 91 or byte == 123 then
        depth += 1
      elseif byte == 93 or byte == 125 then
        if depth > 0 then depth -= 1 end
      elseif byte == 35 and (index == 1 or isSeparation(string.byte(text, index - 1))) then
        return trimRight(string.sub(text, 1, index - 1))
      end
    end
    index += 1
  end

  return trimRight(text)
end

local function significant(index: number): (number?, string?)
  while index <= #Lines do
    local line = Lines[index]
    local content = stripComment(string.sub(line.text, line.indent + 1))
    if content ~= '' then return index, content end
    index += 1
  end
  return nil, nil
end

local function documentMarker(content: string): boolean
  return content == '---' or content == '...'
end

local function mapColon(text: string): number?
  local quote: number? = nil
  local depth = 0
  local index = 1

  while index <= #text do
    local byte = string.byte(text, index)
    if quote == 39 then
      if byte == 39 then
        if string.byte(text, index + 1) == 39 then
          index += 2
          continue
        end
        quote = nil
      end
    elseif quote == 34 then
      if byte == 92 then
        index += 2
        continue
      end

      if byte == 34 then quote = nil end
    else
      if byte == 39 or byte == 34 then
        quote = byte
      elseif byte == 91 or byte == 123 then
        depth += 1
      elseif byte == 93 or byte == 125 then
        if depth > 0 then depth -= 1 end
      elseif byte == 58 and depth == 0 and isSeparation(string.byte(text, index + 1)) then
        return index
      end
    end
    index += 1
  end

  return nil
end

local function looksSequence(content: string): boolean
  return string.byte(content, 1) == 45 and isSeparation(string.byte(content, 2))
end

local function looksExplicitKey(content: string): boolean
  return string.byte(content, 1) == 63 and isSeparation(string.byte(content, 2))
end

local function lineForPosition(position: number, hint: number): number
  local index = math.max(1, hint)
  while index < #Lines and position > Lines[index].finish + (Lines[index].ended and 1 or 0) do
    index += 1
  end
  return index
end

local function encodeCodepoint(value: number, position: number): string
  if value > 0x10ffff or (value >= 0xd800 and value <= 0xdfff) then
    failAt(position, 'escape does not name a Unicode scalar value')
  end
  return utf8.char(value)
end

local function foldLines(lines: { string }): string
  if #lines == 0 then return '' end
  local out = { lines[1] }

  local index = 1
  while index < #lines do
    local nextIndex = index + 1
    if lines[index] ~= '' and lines[nextIndex] ~= '' then
      table.insert(out, ' ')
      table.insert(out, lines[nextIndex])
      index = nextIndex
    elseif lines[index] ~= '' then
      local blanks = 0
      while nextIndex <= #lines and lines[nextIndex] == '' do
        blanks += 1
        nextIndex += 1
      end

      if nextIndex <= #lines then
        table.insert(out, string.rep('\n', math.max(1, blanks)))
        table.insert(out, lines[nextIndex])
        index = nextIndex
      else
        table.insert(out, string.rep('\n', blanks))
        break
      end
    else
      table.insert(out, '\n')
      table.insert(out, lines[nextIndex])
      index = nextIndex
    end
  end

  return table.concat(out)
end

local function parseSignedInteger(text: string, base: number, position: number): any
  local value = integer.fromstring(text, base)
  if value == nil then failAt(position, 'integer is outside the signed 64-bit Luau range') end
  if integer.ge(value, SafeIntegerMin) and integer.le(value, SafeIntegerMax) then return integer.tonumber(value) end
  return value
end

local function resolvePlain(text: string, position: number): any
  if text == '' or text == '~' or text == 'null' or text == 'Null' or text == 'NULL' then return Null end
  if text == 'true' or text == 'True' or text == 'TRUE' then return true end
  if text == 'false' or text == 'False' or text == 'FALSE' then return false end

  if string.match(text, '^[-+]?%d+$') then
    return parseSignedInteger(text, 10, position)
  end

  if string.match(text, '^0o[0-7]+$') then
    return parseSignedInteger(string.sub(text, 3), 8, position)
  end

  if string.match(text, '^0x[%da-fA-F]+$') then
    return parseSignedInteger(string.sub(text, 3), 16, position)
  end

  local lower = string.lower(text)
  if lower == '.inf' or lower == '+.inf' then return Huge end
  if lower == '-.inf' then return -Huge end
  if lower == '.nan' then return 0 / 0 end

  local numeric = string.match(text, '^[-+]?%.%d+[eE][-+]?%d+$')
    or string.match(text, '^[-+]?%.%d+$')
    or string.match(text, '^[-+]?%d+%.%d*[eE][-+]?%d+$')
    or string.match(text, '^[-+]?%d+%.[0-9]*$')
    or string.match(text, '^[-+]?%d+[eE][-+]?%d+$')
  if numeric then
    local value = tonumber(text)
    if not value then failAt(position, 'invalid floating-point value') end
    return value
  end

  return text
end

local function constructTagged(tag: string?, value: any, plain: boolean, position: number): any
  if not tag or tag == '?' then
    if plain then return resolvePlain(value, position) end
    return value
  end

  if tag == '!' then return value end

  local full = tag
  if string.sub(tag, 1, 2) == '!!' then
    full = 'tag:yaml.org,2002:' .. string.sub(tag, 3)
  elseif string.sub(tag, 1, 2) == '!<' and string.sub(tag, -1) == '>' then
    full = string.sub(tag, 3, -2)
  elseif string.sub(tag, 1, 1) == '!' then
    local second = string.find(tag, '!', 2, true)
    if second then
      local handle = string.sub(tag, 1, second)
      local prefix = TagHandles[handle]
      if not prefix then failAt(position, `undefined tag handle {handle}`) end
      full = prefix .. string.sub(tag, second + 1)
    end
  end

  if full == 'tag:yaml.org,2002:str' then return tostring(value == Null and '' or value) end
  if full == 'tag:yaml.org,2002:null' then return Null end

  if full == 'tag:yaml.org,2002:bool' then
    local scalar = string.lower(tostring(value))
    if scalar == 'true' then return true end
    if scalar == 'false' then return false end
    failAt(position, 'invalid !!bool scalar')
  end

  if full == 'tag:yaml.org,2002:int' then
    local scalar = tostring(value)
    if string.match(scalar, '^[-+]?%d+$') then return parseSignedInteger(scalar, 10, position) end
    if string.match(scalar, '^0o[0-7]+$') then return parseSignedInteger(string.sub(scalar, 3), 8, position) end
    if string.match(scalar, '^0x[%da-fA-F]+$') then return parseSignedInteger(string.sub(scalar, 3), 16, position) end
    failAt(position, 'invalid !!int scalar')
  end

  if full == 'tag:yaml.org,2002:float' then
    local scalar = tostring(value)
    local resolved = resolvePlain(scalar, position)
    if type(resolved) == 'number' then return resolved end
    failAt(position, 'invalid !!float scalar')
  end

  if full == 'tag:yaml.org,2002:seq' then
    if type(value) ~= 'table' then failAt(position, '!!seq requires a sequence node') end
    return value
  end

  if full == 'tag:yaml.org,2002:map' then
    if type(value) ~= 'table' then failAt(position, '!!map requires a mapping node') end
    return value
  end

  return { tag = full, value = value }
end

-- Flow parser -----------------------------------------------------------------

local flowAt = 1
local flowCollections = 0

local function skipFlowSpace()
  while flowAt <= length do
    local byte = string.byte(source, flowAt)
    if byte == 32 or byte == 9 or byte == 10 then
      flowAt += 1
    elseif byte == 35 then
      local newline = string.find(source, '\n', flowAt + 1, true)
      flowAt = newline and newline + 1 or length + 1
    else
      break
    end
  end
end

local function parseTagToken(): string
  local start = flowAt
  flowAt += 1
  if string.byte(source, flowAt) == 60 then
    flowAt += 1
    local close = string.find(source, '>', flowAt, true)
    if not close then failAt(start, 'unterminated verbatim tag') end
    local closeAt = close :: number
    flowAt = closeAt + 1
    return string.sub(source, start, closeAt)
  end

  while flowAt <= length do
    local byte = string.byte(source, flowAt)
    if isNameEnd(byte) or byte == 35 then break end
    flowAt += 1
  end
  return string.sub(source, start, flowAt - 1)
end

local function parseAnchorName(indicator: number): string
  local position = flowAt
  if string.byte(source, flowAt) ~= indicator then failAt(flowAt, 'internal anchor parser mismatch') end
  flowAt += 1
  local start = flowAt
  while flowAt <= length do
    local byte = string.byte(source, flowAt)
    if isNameEnd(byte) or byte == 42 or byte == 38 or byte == 33 then break end
    flowAt += 1
  end

  if flowAt == start then failAt(position, 'empty anchor or alias name') end
  return string.sub(source, start, flowAt - 1)
end

local function parseProperties(): (string?, string?)
  local tag: string? = nil
  local anchor: string? = nil
  while true do
    skipFlowSpace()
    local byte = string.byte(source, flowAt)
    if byte == 33 then
      if tag then failAt(flowAt, 'node has more than one tag property') end
      tag = parseTagToken()
    elseif byte == 38 then
      if anchor then failAt(flowAt, 'node has more than one anchor property') end
      anchor = parseAnchorName(38)
    else
      break
    end
  end
  return tag, anchor
end

local function foldQuoted(rawLines: { string }): string
  local cleaned: { string } = {}
  for index, line in rawLines do
    -- White space before a physical line break is presentation separation, but white space before
    -- the closing quote is scalar content and must survive.
    table.insert(cleaned, index < #rawLines and trimRight(line) or line)
  end
  return foldLines(cleaned)
end

local Escapes: { [number]: string } = {
  [48] = '\0',
  [97] = '\7',
  [98] = '\b',
  [116] = '\t',
  [9] = '\t',
  [110] = '\n',
  [118] = '\11',
  [102] = '\f',
  [114] = '\r',
  [101] = '\27',
  [32] = ' ',
  [34] = '"',
  [47] = '/',
  [92] = '\\',
  [78] = utf8.char(0x85),
  [95] = utf8.char(0xa0),
  [76] = utf8.char(0x2028),
  [80] = utf8.char(0x2029),
}

local function parseDoubleQuoted(): string
  local start = flowAt
  flowAt += 1
  local lines: { string }, parts: { string } = {}, {}
  local chunk = flowAt

  while flowAt <= length do
    local byte = string.byte(source, flowAt)
    if byte == 34 then
      if flowAt > chunk then table.insert(parts, string.sub(source, chunk, flowAt - 1)) end
      table.insert(lines, table.concat(parts))
      flowAt += 1
      return foldQuoted(lines)
    end

    if byte == 92 then
      if flowAt > chunk then table.insert(parts, string.sub(source, chunk, flowAt - 1)) end
      local escapeAt = flowAt
      flowAt += 1
      local escaped = string.byte(source, flowAt)
      if escaped == 10 then
        flowAt += 1
        while string.byte(source, flowAt) == 32 or string.byte(source, flowAt) == 9 do flowAt += 1 end
        chunk = flowAt
        continue
      end

      local direct = escaped and Escapes[escaped]
      if direct then
        table.insert(parts, direct)
        flowAt += 1
      elseif escaped == 120 or escaped == 117 or escaped == 85 then
        local digits = escaped == 120 and 2 or (escaped == 117 and 4 or 8)
        local value = Common.hex(source, flowAt + 1, digits)
        if not value then failAt(escapeAt, 'invalid hexadecimal escape in double-quoted scalar') end
        table.insert(parts, encodeCodepoint(value :: number, escapeAt))
        flowAt += digits + 1
      else
        failAt(escapeAt, 'invalid escape in double-quoted scalar')
      end
      chunk = flowAt
      continue
    end

    if byte == 10 then
      if flowAt > chunk then table.insert(parts, string.sub(source, chunk, flowAt - 1)) end
      table.insert(lines, table.concat(parts))
      table.clear(parts)
      flowAt += 1
      while string.byte(source, flowAt) == 32 or string.byte(source, flowAt) == 9 do flowAt += 1 end
      chunk = flowAt
      continue
    end

    if byte and byte < 32 and byte ~= 9 then failAt(flowAt, 'control character in double-quoted scalar') end
    flowAt += 1
  end

  failAt(start, 'unterminated double-quoted scalar')
  error('unreachable', 0)
end

local function parseSingleQuoted(): string
  local start = flowAt
  flowAt += 1
  local lines: { string }, parts: { string } = {}, {}
  local chunk = flowAt

  while flowAt <= length do
    local byte = string.byte(source, flowAt)
    if byte == 39 then
      if string.byte(source, flowAt + 1) == 39 then
        if flowAt > chunk then table.insert(parts, string.sub(source, chunk, flowAt - 1)) end
        table.insert(parts, "'")
        flowAt += 2
        chunk = flowAt
        continue
      end

      if flowAt > chunk then table.insert(parts, string.sub(source, chunk, flowAt - 1)) end
      table.insert(lines, table.concat(parts))
      flowAt += 1
      return foldQuoted(lines)
    end

    if byte == 10 then
      if flowAt > chunk then table.insert(parts, string.sub(source, chunk, flowAt - 1)) end
      table.insert(lines, table.concat(parts))
      table.clear(parts)
      flowAt += 1
      while string.byte(source, flowAt) == 32 or string.byte(source, flowAt) == 9 do flowAt += 1 end
      chunk = flowAt
      continue
    end

    if byte and byte < 32 and byte ~= 9 then failAt(flowAt, 'control character in single-quoted scalar') end
    flowAt += 1
  end

  failAt(start, 'unterminated single-quoted scalar')
  error('unreachable', 0)
end

local parseFlowNode: (boolean, number) -> any

local function setMappingPair(map: { [any]: any }, key: any, value: any, position: number)
  if key == nil then key = Null end
  if type(key) == 'number' and key ~= key then failAt(position, 'NaN cannot be represented as a Luau table key') end
  if map[key] ~= nil then failAt(position, 'duplicate mapping key') end
  map[key] = value
end

local function parseFlowPlain(stopColon: boolean): (string, number)
  local start = flowAt
  local lineStart = flowAt
  local lines: { string } = {}

  while flowAt <= length do
    local byte = string.byte(source, flowAt)
    if byte == 44 or byte == 93 or byte == 125 then break end
    if byte == 35 and (flowAt == lineStart or isSeparation(string.byte(source, flowAt - 1))) then break end

    if byte == 58 then
      local nextByte = string.byte(source, flowAt + 1)
      if stopColon and (isSeparation(nextByte) or isFlowDelimiter(nextByte)) then break end
      if isFlowDelimiter(nextByte) then break end
    end

    if byte == 91 or byte == 123 then break end

    if byte == 10 then
      if flowCollections == 0 then break end
      table.insert(lines, trimRight(string.sub(source, lineStart, flowAt - 1)))
      flowAt += 1
      while string.byte(source, flowAt) == 32 or string.byte(source, flowAt) == 9 do flowAt += 1 end
      lineStart = flowAt
      continue
    end
    flowAt += 1
  end

  local tail = trimRight(string.sub(source, lineStart, flowAt - 1))
  if #lines == 0 then
    if tail == '' then failAt(start, 'expected scalar content') end
    return tail, start
  end

  if tail ~= '' then table.insert(lines, tail) end
  local value = foldLines(lines)
  if value == '' then failAt(start, 'expected scalar content') end
  return value, start
end

local function registerAnchor(anchor: string?, value: any, position: number)
  if not anchor then return end
  if Anchors[anchor] ~= nil then failAt(position, `anchor &{anchor} is already defined`) end
  Anchors[anchor] = value
end

local function parseFlowSequence(tag: string?, anchor: string?, depth: number, position: number): any
  if depth > maxDepth then failAt(position, 'maximum nesting depth exceeded') end
  local list: { any } = {}
  registerAnchor(anchor, list, position)
  flowCollections += 1
  flowAt += 1
  skipFlowSpace()

  if string.byte(source, flowAt) == 93 then
    flowAt += 1
    flowCollections -= 1
    return constructTagged(tag, list, false, position)
  end

  while true do
    local itemStart = flowAt
    local item = parseFlowNode(true, depth + 1)
    skipFlowSpace()

    if string.byte(source, flowAt) == 58 then
      flowAt += 1
      skipFlowSpace()
      local map: { [any]: any } = {}
      local value
      local nextByte = string.byte(source, flowAt)
      if nextByte == 44 or nextByte == 93 then value = Null else value = parseFlowNode(false, depth + 1) end
      setMappingPair(map, item, value, itemStart)
      item = map
    end
    table.insert(list, item)

    skipFlowSpace()
    local byte = string.byte(source, flowAt)
    if byte == 93 then
      flowAt += 1
      break
    end

    if byte ~= 44 then failAt(flowAt, "expected ',' or ']' in flow sequence") end
    flowAt += 1
    skipFlowSpace()
    if string.byte(source, flowAt) == 93 then
      flowAt += 1
      break
    end
  end

  flowCollections -= 1
  return constructTagged(tag, list, false, position)
end

local function parseFlowMapping(tag: string?, anchor: string?, depth: number, position: number): any
  if depth > maxDepth then failAt(position, 'maximum nesting depth exceeded') end
  local map: { [any]: any } = {}
  registerAnchor(anchor, map, position)
  flowCollections += 1
  flowAt += 1
  skipFlowSpace()

  if string.byte(source, flowAt) == 125 then
    flowAt += 1
    flowCollections -= 1
    return constructTagged(tag, map, false, position)
  end

  while true do
    local pairAt = flowAt
    local explicit = string.byte(source, flowAt) == 63 and isSeparation(string.byte(source, flowAt + 1))
    if explicit then
      flowAt += 1
      skipFlowSpace()
    end

    local key: any = Null
    local byte = string.byte(source, flowAt)
    if byte ~= 58 and byte ~= 44 and byte ~= 125 then key = parseFlowNode(true, depth + 1) end
    skipFlowSpace()

    local value: any = Null
    if string.byte(source, flowAt) == 58 then
      flowAt += 1
      skipFlowSpace()
      byte = string.byte(source, flowAt)
      if byte ~= 44 and byte ~= 125 then value = parseFlowNode(false, depth + 1) end
    elseif not explicit then
      failAt(flowAt, "expected ':' in flow mapping")
    end
    setMappingPair(map, key, value, pairAt)

    skipFlowSpace()
    byte = string.byte(source, flowAt)
    if byte == 125 then
      flowAt += 1
      break
    end

    if byte ~= 44 then failAt(flowAt, "expected ',' or '}' in flow mapping") end
    flowAt += 1
    skipFlowSpace()
    if string.byte(source, flowAt) == 125 then
      flowAt += 1
      break
    end
  end

  flowCollections -= 1
  return constructTagged(tag, map, false, position)
end

parseFlowNode = function(stopColon: boolean, depth: number): any
  if depth > maxDepth then failAt(flowAt, 'maximum nesting depth exceeded') end
  skipFlowSpace()
  local position = flowAt
  local tag, anchor = parseProperties()
  skipFlowSpace()
  position = math.min(position, flowAt)

  local byte = string.byte(source, flowAt)
  if byte == 42 then
    if tag or anchor then failAt(flowAt, 'alias node may not have node properties') end
    local name = parseAnchorName(42)
    local value = Anchors[name]
    if value == nil then failAt(position, `alias *{name} refers to an unknown anchor`) end
    return value
  end

  if byte == 91 then return parseFlowSequence(tag, anchor, depth, position) end
  if byte == 123 then return parseFlowMapping(tag, anchor, depth, position) end

  local value: any
  local plain = false
  if byte == 34 then
    value = parseDoubleQuoted()
  elseif byte == 39 then
    value = parseSingleQuoted()
  else
    local raw, rawAt = parseFlowPlain(stopColon)
    value = raw
    plain = true
    position = rawAt
  end
  value = constructTagged(tag, value, plain, position)
  registerAnchor(anchor, value, position)
  return value
end

local function ensureInlineFinished(lineIndex: number): number
  local endingLine = lineForPosition(flowAt, lineIndex)
  local line = Lines[endingLine]
  local restStart = flowAt - line.start + 1
  local rest = string.sub(line.text, math.max(1, restStart))
  local clean = stripComment(rest)
  if trim(clean) ~= '' then failAt(flowAt, 'unexpected content after node') end
  return endingLine + 1
end

-- Block parser ----------------------------------------------------------------

type BlockNodeParser = (number, number, number, string?, string?) -> (any, number)
local parseBlockNodeImpl: BlockNodeParser = nil :: any
local function parseBlockNode(index: number, indent: number, depth: number, tag: string?, anchor: string?): (any, number)
  return parseBlockNodeImpl(index, indent, depth, tag, anchor)
end

local function parseBlockScalar(index: number, indicatorColumn: number, token: string): (string, number)
  local line = Lines[index]
  local style = string.sub(token, 1, 1)
  local modifiers = stripComment(string.sub(token, 2))
  local chomp = ''
  local explicit: number? = nil

  for offset = 1, #modifiers do
    local byte = string.byte(modifiers, offset)
    if byte == 43 or byte == 45 then
      if chomp ~= '' then failLine(index, indicatorColumn + offset, 'duplicate block scalar chomping indicator') end
      chomp = string.char(byte)
    elseif byte and byte >= 49 and byte <= 57 then
      if explicit then failLine(index, indicatorColumn + offset, 'duplicate block scalar indentation indicator') end
      explicit = byte - 48
    elseif byte == 32 or byte == 9 then
      local trailing = trim(string.sub(modifiers, offset))
      if trailing ~= '' then failLine(index, indicatorColumn + offset, 'invalid block scalar header') end
      break
    else
      failLine(index, indicatorColumn + offset, 'invalid block scalar header')
    end
  end

  local parentIndent = line.indent
  local contentIndent = explicit and parentIndent + explicit or nil
  local scan = index + 1
  if not contentIndent then
    while scan <= #Lines do
      local candidate = Lines[scan]
      if candidate.text ~= '' and trim(candidate.text) ~= '' then
        if candidate.indent <= parentIndent then
          contentIndent = parentIndent + 1
        else
          contentIndent = candidate.indent
        end
        break
      end
      scan += 1
    end
    contentIndent = contentIndent or parentIndent + 1
  end

  local values: { string } = {}
  local more: { boolean } = {}
  local ended = false
  scan = index + 1
  while scan <= #Lines do
    local candidate = Lines[scan]
    local blank = trim(candidate.text) == ''
    if not blank and candidate.indent < contentIndent then break end
    if not blank and candidate.indent <= parentIndent then break end

    local value = blank and '' or string.sub(candidate.text, contentIndent + 1)
    table.insert(values, value)
    table.insert(more, not blank and candidate.indent > contentIndent)
    ended = candidate.ended
    scan += 1
  end

  local out: { string } = {}
  if style == '|' then
    for index, value in values do
      table.insert(out, value)
      if index < #values or ended then table.insert(out, '\n') end
    end
  else
    for index, value in values do
      table.insert(out, value)
      if index < #values then
        local nextValue = values[index + 1]
        if more[index] or more[index + 1] then
          table.insert(out, '\n')
        elseif value ~= '' and nextValue ~= '' then
          table.insert(out, ' ')
        elseif value == '' then
          table.insert(out, '\n')
        end
      elseif ended then
        table.insert(out, '\n')
      end
    end
  end

  local value = table.concat(out)
  if chomp == '-' then
    value = string.gsub(value, '\n+$', '')
  elseif chomp == '' then
    value = string.gsub(value, '\n+$', '')
    if #values > 0 and ended then value ..= '\n' end
  end
  return value, scan
end

local function startsSpecialInline(text: string): boolean
  local byte = string.byte(text, 1)
  return byte == 34 or byte == 39 or byte == 91 or byte == 123 or byte == 42 or byte == 33 or byte == 38
end

local function propertyOnly(text: string): (string?, string?, boolean)
  local rest = trim(stripComment(text))
  local tag: string? = nil
  local anchor: string? = nil

  while rest ~= '' do
    local first = string.sub(rest, 1, 1)
    if first ~= '!' and first ~= '&' then return tag, anchor, false end
    local token: string
    if string.sub(rest, 1, 2) == '!<' then
      local close = string.find(rest, '>', 3, true)
      if not close then return tag, anchor, false end
      token = string.sub(rest, 1, close)
      rest = trim(string.sub(rest, close + 1))
    else
      local space = string.find(rest, '[ \t]')
      token = space and string.sub(rest, 1, space - 1) or rest
      rest = space and trim(string.sub(rest, space + 1)) or ''
    end

    if first == '!' then
      if tag then return tag, anchor, false end
      tag = token
    else
      if anchor or #token == 1 then return tag, anchor, false end
      anchor = string.sub(token, 2)
    end
  end

  return tag, anchor, tag ~= nil or anchor ~= nil
end

local function parsePlainBlock(first: string, index: number, parentIndent: number, position: number): (any, number)
  local lines = { trim(first) }
  local scan = index + 1

  while scan <= #Lines do
    local candidate = Lines[scan]
    local clean = stripComment(string.sub(candidate.text, candidate.indent + 1))
    if documentMarker(clean) or string.byte(clean, 1) == 37 then break end

    if clean == '' then
      local nextIndex, nextContent = significant(scan + 1)
      if not nextIndex or Lines[nextIndex].indent <= parentIndent or (nextContent and documentMarker(nextContent)) then break end
      table.insert(lines, '')
      scan += 1
      continue
    end

    if candidate.indent <= parentIndent then break end
    table.insert(lines, trim(clean))
    scan += 1
  end

  return resolvePlain(foldLines(lines), position), scan
end

local function parseInlineBlock(text: string, index: number, parentIndent: number, depth: number): (any, number)
  local line = Lines[index]
  local leading = #text - #string.gsub(text, '^[ \t]+', '')
  local clean = string.sub(text, leading + 1)
  local position = line.start + (#line.text - #text) + leading

  if string.sub(clean, 1, 1) == '|' or string.sub(clean, 1, 1) == '>' then
    local value, nextIndex = parseBlockScalar(index, position - line.start + 1, clean)
    return value, nextIndex
  end

  if not startsSpecialInline(clean) then
    return parsePlainBlock(stripComment(clean), index, parentIndent, position)
  end

  flowAt = position
  local value = parseFlowNode(false, depth)
  local nextIndex = ensureInlineFinished(index)
  return value, nextIndex
end

local function parseKeyText(text: string, index: number, depth: number): any
  local clean = trim(text)
  if clean == '' then return Null end
  local line = Lines[index]
  local relative = string.find(line.text, clean, 1, true) or 1
  local position = line.start + relative - 1

  if startsSpecialInline(clean) then
    flowAt = position
    local key = parseFlowNode(true, depth)
    local ending = lineForPosition(flowAt, index)
    if ending ~= index then failAt(position, 'implicit mapping key must be on one line') end
    return key
  end
  return resolvePlain(clean, position)
end

local function parseBlockSequence(index: number, indent: number, depth: number, tag: string?, anchor: string?, firstContent: string?): (any, number)
  local list: { any } = {}
  local position = Lines[index].start + indent
  registerAnchor(anchor, list, position)
  local scan = index

  while scan <= #Lines do
    local found: number?
    local content: string?
    local compactFirst = firstContent ~= nil
    if firstContent then
      found, content = scan, firstContent
      firstContent = nil
    else
      found, content = significant(scan)
    end

    if not found or not content then
      scan = #Lines + 1
      break
    end
    local foundAt = found :: number
    local contentText = content :: string
    local line = Lines[foundAt]
    if (not compactFirst and line.indent ~= indent) or not looksSequence(contentText) then break end
    if documentMarker(contentText) then break end

    local rest = string.sub(contentText, 2)
    if string.sub(rest, 1, 1) == ' ' or string.sub(rest, 1, 1) == '\t' then rest = string.sub(rest, 2) end
    rest = stripComment(rest)

    if trim(rest) == '' then
      local child, childContent = significant(foundAt + 1)
      if child and childContent and not documentMarker(childContent) and Lines[child].indent > indent then
        local value, nextIndex = parseBlockNode(child, Lines[child].indent, depth + 1, nil, nil)
        table.insert(list, value)
        scan = nextIndex
      else
        table.insert(list, Null)
        scan = foundAt + 1
      end
      continue
    end

    -- Compact block mapping: `- key: value`, followed by more entries at the compact mapping's
    -- indentation. Parse it through the mapping machinery by supplying the first entry explicitly.
    if looksSequence(rest) then
      local nested, nextIndex = parseBlockSequence(foundAt, indent + 2, depth + 1, nil, nil, rest)
      table.insert(list, nested)
      scan = nextIndex
      continue
    end

    local propertyTag, propertyAnchor, onlyProperties = propertyOnly(rest)
    if onlyProperties then
      local child, childContent = significant(foundAt + 1)
      if child and childContent and Lines[child].indent > indent then
        local value, nextIndex = parseBlockNode(child, Lines[child].indent, depth + 1, propertyTag, propertyAnchor)
        table.insert(list, value)
        scan = nextIndex
      else
        local value = constructTagged(propertyTag, Null, false, line.start + indent)
        registerAnchor(propertyAnchor, value, line.start + indent)
        table.insert(list, value)
        scan = foundAt + 1
      end
      continue
    end

    local colon = mapColon(rest)
    if colon and not startsSpecialInline(trim(rest)) then
      local map: { [any]: any } = {}
      local key = parseKeyText(string.sub(rest, 1, colon - 1), foundAt, depth + 1)
      local rhs = string.sub(rest, colon + 1)
      local value: any
      local nextIndex: number
      if trim(stripComment(rhs)) == '' then
        local child, childContent = significant(foundAt + 1)
        if child and childContent and Lines[child].indent > indent then
          value, nextIndex = parseBlockNode(child, Lines[child].indent, depth + 2, nil, nil)
        else
          value, nextIndex = Null, foundAt + 1
        end
      else
        value, nextIndex = parseInlineBlock(rhs, foundAt, indent + 2, depth + 2)
      end
      setMappingPair(map, key, value, position)

      local compactIndent: number? = nil
      local look, lookContent = significant(nextIndex)
      if look and lookContent and Lines[look].indent > indent then compactIndent = Lines[look].indent end

      if compactIndent then
        local moreMap, after = parseBlockNode(look :: number, compactIndent, depth + 1, nil, nil)
        if type(moreMap) ~= 'table' then failLine(look :: number, compactIndent + 1, 'compact mapping continuation must be a mapping') end
        for otherKey, otherValue in moreMap do setMappingPair(map, otherKey, otherValue, Lines[look :: number].start) end
        nextIndex = after
      end
      table.insert(list, map)
      scan = nextIndex
    else
      local value, nextIndex = parseInlineBlock(rest, foundAt, indent, depth + 1)
      table.insert(list, value)
      scan = nextIndex
    end
  end

  return constructTagged(tag, list, false, position), scan
end

local function parseBlockMapping(index: number, indent: number, depth: number, tag: string?, anchor: string?): (any, number)
  local map: { [any]: any } = {}
  local position = Lines[index].start + indent
  registerAnchor(anchor, map, position)
  local scan = index

  while scan <= #Lines do
    local found, content = significant(scan)
    if not found or not content then
      scan = #Lines + 1
      break
    end
    local line = Lines[found]
    if line.indent ~= indent or looksSequence(content) or documentMarker(content) then break end

    local key: any
    local rhs: string
    local pairAt = line.start + indent

    if looksExplicitKey(content) then
      local afterQuestion = stripComment(string.sub(content, 2))
      key = trim(afterQuestion) == '' and Null or parseKeyText(afterQuestion, found, depth + 1)
      local valueLine, valueContent = significant(found + 1)
      if not valueLine or not valueContent or Lines[valueLine].indent ~= indent or string.byte(valueContent, 1) ~= 58
        or not isSeparation(string.byte(valueContent, 2)) then
        setMappingPair(map, key, Null, pairAt)
        scan = found + 1
        continue
      end
      rhs = string.sub(valueContent, 2)
      found = valueLine
      line = Lines[found]
    else
      local colon = mapColon(content)
      if not colon then break end
      key = parseKeyText(string.sub(content, 1, colon - 1), found, depth + 1)
      rhs = string.sub(content, colon + 1)
    end

    local cleanRhs = stripComment(rhs)
    local value: any
    local nextIndex: number
    local propertyTag, propertyAnchor, onlyProperties = propertyOnly(cleanRhs)
    if onlyProperties then
      local child, childContent = significant(found + 1)
      if child and childContent and not documentMarker(childContent) and Lines[child].indent > indent then
        value, nextIndex = parseBlockNode(child, Lines[child].indent, depth + 1, propertyTag, propertyAnchor)
      else
        value = constructTagged(propertyTag, Null, false, line.start + indent)
        registerAnchor(propertyAnchor, value, line.start + indent)
        nextIndex = found + 1
      end
    elseif trim(cleanRhs) == '' then
      local child, childContent = significant(found + 1)
      if child and childContent and not documentMarker(childContent) then
        local childIndent = Lines[child].indent
        if childIndent > indent or (childIndent == indent and looksSequence(childContent)) then
          value, nextIndex = parseBlockNode(child, childIndent, depth + 1, nil, nil)
        else
          value, nextIndex = Null, found + 1
        end
      else
        value, nextIndex = Null, found + 1
      end
    else
      value, nextIndex = parseInlineBlock(rhs, found, indent, depth + 1)
    end
    setMappingPair(map, key, value, pairAt)
    scan = nextIndex
  end

  return constructTagged(tag, map, false, position), scan
end

local function parsePropertyPrefix(text: string, index: number): (string?, string?, string)
  local line = Lines[index]
  local base = line.start + line.indent
  flowAt = base
  local tag, anchor = parseProperties()
  local consumed = flowAt - base
  return tag, anchor, trim(string.sub(text, consumed + 1))
end

parseBlockNodeImpl = function(index: number, indent: number, depth: number, inheritedTag: string?, inheritedAnchor: string?): (any, number)
  if depth > maxDepth then failLine(index, indent + 1, 'maximum nesting depth exceeded') end
  local line = Lines[index]
  local content = stripComment(string.sub(line.text, indent + 1))
  if content == '' then return Null, index + 1 end

  local tag, anchor = inheritedTag, inheritedAnchor
  if string.byte(content, 1) == 33 or string.byte(content, 1) == 38 then
    local originalContent = content
    local parsedTag, parsedAnchor, rest = parsePropertyPrefix(content, index)
    tag = parsedTag or tag
    anchor = parsedAnchor or anchor
    content = rest
    if content == '' then
      local child, childContent = significant(index + 1)
      if child and childContent and Lines[child].indent > indent then
        return parseBlockNode(child, Lines[child].indent, depth + 1, tag, anchor)
      end
      local value = constructTagged(tag, Null, false, line.start + indent)
      registerAnchor(anchor, value, line.start + indent)
      return value, index + 1
    end

    if not looksSequence(content) and not looksExplicitKey(content) and not mapColon(content) then
      return parseInlineBlock(originalContent, index, indent - 1, depth)
    end
  end

  if looksSequence(content) then return parseBlockSequence(index, indent, depth, tag, anchor) end
  if looksExplicitKey(content) or mapColon(content) then return parseBlockMapping(index, indent, depth, tag, anchor) end

  local value, nextIndex = parseInlineBlock(content, index, indent - 1, depth)
  if tag then value = constructTagged(tag :: string, value, false, line.start + indent) end
  registerAnchor(anchor, value, line.start + indent)
  return value, nextIndex
end

local function resetDocument()
  table.clear(Anchors)
  table.clear(TagHandles)
  TagHandles['!!'] = 'tag:yaml.org,2002:'
  TagHandles['!'] = '!'
end

local function parseDirective(index: number, content: string): number
  if string.sub(content, 1, 5) == '%YAML' and isSeparation(string.byte(content, 6)) then
    local version = trim(string.sub(content, 6))
    if version ~= '1.2' then failLine(index, 1, `unsupported YAML version {version}`) end
  elseif string.sub(content, 1, 4) == '%TAG' and isSeparation(string.byte(content, 5)) then
    local rest = trim(string.sub(content, 5))
    local handle, prefix = string.match(rest, '^(%S+)%s+(%S+)$')
    if not handle or not prefix or string.sub(handle, 1, 1) ~= '!' or string.sub(handle, -1) ~= '!' then
      failLine(index, 1, 'invalid %TAG directive')
    end

    local handleValue, prefixValue = handle :: string, prefix :: string
    if TagHandles[handleValue] and handleValue ~= '!' and handleValue ~= '!!' then failLine(index, 1, `duplicate tag handle {handleValue}`) end
    TagHandles[handleValue] = prefixValue
  else
    failLine(index, 1, 'unknown YAML directive')
  end
  return index + 1
end

local function parseDocuments(text: string, options: Options?): { any }
  source = normalize(text)
  length = #source
  maxDepth = options and options.maxDepth or 512
  flowCollections = 0
  if maxDepth < 1 or maxDepth ~= math.floor(maxDepth) then error('YAML: maxDepth must be a positive integer', 2) end
  buildLines()

  local documents: { any } = {}
  local index = 1

  while true do
    resetDocument()
    local sawDirective = false
    while true do
      local found, content = significant(index)
      if not found or not content then return documents end

      if string.byte(content, 1) ~= 37 then
        index = found
        break
      end

      sawDirective = true
      index = parseDirective(found, content)
    end

    local found, content = significant(index)
    if not found or not content then return documents end

    if sawDirective then
      if content ~= '---' then failLine(found, 1, "directives must be followed by '---'") end
      index = found + 1
    elseif content == '---' then
      index = found + 1
    elseif content == '...' then
      index = found + 1
      table.insert(documents, Null)
      continue
    end

    found, content = significant(index)
    if not found or not content then
      table.insert(documents, Null)
      return documents
    end

    if content == '...' or content == '---' then
      table.insert(documents, Null)
      if content == '...' then index = found + 1 else index = found end
    else
      local value, nextIndex = parseBlockNode(found, Lines[found].indent, 1, nil, nil)
      table.insert(documents, value)
      index = nextIndex
    end

    local marker, markerContent = significant(index)
    if not marker or not markerContent then return documents end

    if markerContent == '...' then
      index = marker + 1
      local nextLine, nextContent = significant(index)
      if nextLine and nextContent and string.byte(nextContent, 1) == 37 then
        index = nextLine
      end
    elseif markerContent == '---' then
      index = marker
    elseif string.byte(markerContent, 1) == 37 then
      failLine(marker, 1, "a directive after a document requires an explicit '...' document end")
    else
      failLine(marker, Lines[marker].indent + 1, 'unexpected content after YAML document')
    end
  end
end

local function decodeAll(text: string, options: Options?): { any }
  return parseDocuments(text, options)
end

local function decode(text: string, options: Options?): any
  local documents = parseDocuments(text, options)
  if #documents == 0 then return Null end
  if #documents ~= 1 then error(`YAML: expected one document, got {#documents}; use decodeAll`, 2) end
  return documents[1]
end

--- A value built with `@jess/data/value` as one YAML document in block style, as serde_yaml
--- writes it: plain scalars where a reader takes them back as what they were, quoted otherwise.
local function encode(value: Values.Value): string return Values.encode(value, 'yaml') end

return table.freeze {
  version = '1.2.2',
  null = Null,
  decode = decode,
  decodeAll = decodeAll,
  encode = encode,
}
