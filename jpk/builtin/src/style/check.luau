--!strict
--!native
-- The style checker: one parse through @dream/luau with the token stream, one pass over the tokens
-- for the layout of every line, and one walk over the tree for everything else. The rules are the
-- ones site/content/docs/contributing/luau-style.md describes, s3stash's lua-code-style skill and
-- its YAMATO commit being the source of truth.
local luau = require '@dream/luau'
local rules = require './rules'

type Node = dream_luau_Node
type Expr = dream_luau_Expr
type Stat = dream_luau_Stat
type Block = dream_luau_StatBlock
type Local = dream_luau_Local
type Type = dream_luau_Type
type TypePack = dream_luau_TypePack
type Function = dream_luau_ExprFunction

export type Finding = rules.Finding
export type Options = rules.Options

local TokenKinds = luau.tokenKinds
local CommentKind, BlockCommentKind = TokenKinds.comment, TokenKinds.blockComment

-- Line flags, one byte per line.
local HasToken, ContinuesText = 1, 2

-- Luau's global functions and libraries: a local that only renames one of these is hoisting.
local Builtins: { [string]: boolean } = {
  assert = true,
  bit32 = true,
  buffer = true,
  collectgarbage = true,
  coroutine = true,
  debug = true,
  error = true,
  gcinfo = true,
  getfenv = true,
  getmetatable = true,
  ipairs = true,
  math = true,
  newproxy = true,
  next = true,
  os = true,
  pairs = true,
  pcall = true,
  print = true,
  rawequal = true,
  rawget = true,
  rawlen = true,
  rawset = true,
  select = true,
  setfenv = true,
  setmetatable = true,
  string = true,
  table = true,
  tonumber = true,
  tostring = true,
  type = true,
  typeof = true,
  unpack = true,
  utf8 = true,
  vector = true,
  xpcall = true,
}

-- Luau's named types that are never false; tables and functions are their own syntax.
local ValueTypes: { [string]: boolean } = {
  buffer = true,
  integer = true,
  ['nil'] = true,
  number = true,
  string = true,
  thread = true,
  userdata = true,
  vector = true,
}

-- LuaLS's built-in types that are never false.
local LuaLsValueTypes: { [string]: boolean } = {
  integer = true,
  number = true,
  string = true,
  table = true,
}

-- Library members that are values, not functions: naming one is naming a constant.
local LibraryConstants: { [string]: boolean } = {
  ['math.huge'] = true,
  ['math.pi'] = true,
  ['utf8.charpattern'] = true,
  ['vector.one'] = true,
  ['vector.zero'] = true,
}

-- Library calls that never return false, so comparing their result with nil is a truthiness test.
local NeverFalse: { [string]: boolean } = {
  ['string.find'] = true,
  ['string.match'] = true,
  ['string.byte'] = true,
  ['table.find'] = true,
  tonumber = true,
}

-- Library calls that change the table they're handed first.
local Mutators: { [string]: boolean } = {
  ['table.clear'] = true,
  ['table.insert'] = true,
  ['table.move'] = true,
  ['table.remove'] = true,
  ['table.sort'] = true,
  setmetatable = true,
}

-- What a single letter usually stands for, for the message. Every single-letter name is reported.
local SingleLetters: { [string]: string } = {
  ['a'] = 'left, first, ...',
  ['b'] = 'right, second, ...',
  ['c'] = 'third, count, ...',
  ['e'] = 'event, entry, error, ...',
  ['f'] = 'a name for what the function does',
  ['i'] = 'index',
  ['j'] = 'index, or what it counts',
  ['k'] = 'key, or what the key is',
  ['n'] = 'count',
  ['p'] = 'position, point, ...',
  ['s'] = 'text, scale, ...',
  ['t'] = 'table, time, triangle, ...',
  ['v'] = 'value, or what the value is',
  ['x'] = 'the axis or column it is',
  ['y'] = 'the axis or row it is',
  ['z'] = 'the axis it is',
}

-- Abbreviations the style spells out: `record`, not `rec`.
local Abbreviations: { [string]: string } = {
  ['arr'] = 'list, or what it lists',
  ['btn'] = 'button',
  ['buf'] = 'buffer, or what it holds',
  ['cb'] = 'callback, or what it does',
  ['cnt'] = 'count',
  ['ctx'] = 'context',
  ['dst'] = 'destination',
  ['el'] = 'element',
  ['elem'] = 'element',
  ['fn'] = 'a name for what the function does',
  ['idx'] = 'index',
  ['msg'] = 'message',
  ['num'] = 'number, or what it counts',
  ['obj'] = 'object, or what it is',
  ['rec'] = 'record',
  ['res'] = 'result',
  ['ret'] = 'result',
  ['src'] = 'source',
  ['str'] = 'text, or what it says',
  ['tbl'] = 'a name for what the table holds',
  ['tmp'] = 'a name for what it holds',
  ['val'] = 'value',
}

local function isCamel(name: string): boolean
  return string.find(name, '^_?[a-z][%w]*$') ~= nil or name == '_'
end

-- All capitals is PascalCase only for an acronym: three letters at most (`UI`, `URL`), or a name
-- with a digit in it (`H3UI`). `IDENTITY` is a constant shouting.
local function isPascal(name: string): boolean
  if not string.find(name, '^[A-Z][%w]*$') then return false end

  return #name <= 3 or string.find(name, '[%l%d]') ~= nil
end

-- `DOCUMENT_VERSION` and `document_version` as `DocumentVersion`.
local function pascalCase(name: string): string
  local parts = {}

  for word in string.gmatch(name, '[^_]+') do
    local lower = if string.upper(word) == word then string.lower(word) else word
    table.insert(parts, string.upper(string.sub(lower, 1, 1)) .. string.sub(lower, 2))
  end

  return table.concat(parts)
end

-- The state of one check. Module upvalues rather than a table threaded through every call: the walk
-- reads them on every node.
local source = ''
local options: Options = {}
local findings: { Finding } = {}
local lineStarts: { number } = {}
local lineCount = 0
local lineFlags = buffer.create(0)
local lineFirst = buffer.create(0)
local tokens = buffer.create(0)
local tokenCount = 0
local jessProfile, naming, guards, noAny = true, true, true, false
-- The rules the caller asked for, when it named them.
local only: { [string]: boolean }? = nil

-- What the walk learns about locals, keyed by the one table each local is.
local reassigned: { [Local]: boolean }, mutated: { [Local]: boolean } = {}, {}
-- Locals initialized with a literal: 'table' for a table of literals, 'value' for anything else.
local constants: { [Local]: 'table' | 'value' } = {}
local nonBoolean: { [Local]: boolean } = {}
local pascalLocals: { Local } = {}
local moduleLocals: { [Local]: boolean } = {}

-- The statements the walk is inside, innermost last: where each starts.
local enclosingLines: { number }, enclosingColumns: { number } = {}, {}
-- Where the run of adjacent statements holding a protected call before each enclosing statement
-- starts, or 0: one comment above the run speaks for every call in it.
local runLines: { number }, runColumns: { number } = {}, {}
local depth = 0
-- Whether the statement being walked holds a pcall or an xpcall.
local protectedSeen = false
-- Whether the statement being walked holds a table constructor or function spanning several lines.
local nestedBlock = false
-- Whether the expression being walked is tested for truth: a condition, or a part of one.
local testing = false
-- The body of the function a module returns, when it returns one: a load-order tool's
-- `return function(state, openmw)`. Its locals are the module's.
local moduleBody: Block? = nil

local function report(
  rule: string,
  node: { line: number, column: number, endLine: number, endColumn: number },
  message: string
)
  if only and not only[rule] then return end

  local definition = rules.Rules[rule]

  table.insert(findings, {
    rule = rule,
    severity = definition.severity,
    message = message,
    line = node.line,
    column = node.column,
    endLine = node.endLine,
    endColumn = node.endColumn,
  })
end

local function reportAt(
  rule: string,
  line: number,
  column: number,
  endColumn: number,
  message: string
)
  report(rule, { line = line, column = column, endLine = line, endColumn = endColumn }, message)
end

-- The 1-based source index of a position.
local function offset(line: number, column: number): number return lineStarts[line] + column - 1 end

-- The index of the token starting at or after `at`, 0-based in records; tokenCount when none does.
local function tokenAt(at: number): number
  local low, high = 0, tokenCount

  while low < high do
    local middle = (low + high) // 2

    if buffer.readu32(tokens, middle * 12 + 4) < at then
      low = middle + 1
    else
      high = middle
    end
  end

  return low
end

local function lineOf(at: number): number
  local low, high = 1, lineCount

  while low < high do
    local middle = (low + high + 1) // 2

    if lineStarts[middle] <= at then
      low = middle
    else
      high = middle - 1
    end
  end

  return low
end

-- Whether a line between `first` and `last`, both excluded, holds nothing: no token, no comment.
local function blankBetween(first: number, last: number): boolean
  for line = first + 1, last - 1 do
    if buffer.readu8(lineFlags, line - 1) == 0 then return true end
  end

  return false
end

-- Whether a comment that says something ends right above the source index `at`, with no blank line
-- between. Directives (`--!strict`) and LuaLS annotations (`---@type`) say nothing about the code.
local function commentBefore(at: number): boolean
  local index = tokenAt(at) - 1
  if index < 0 then return false end

  local kind = buffer.readu32(tokens, index * 12)
  if kind ~= CommentKind and kind ~= BlockCommentKind then return false end

  local first, last = buffer.readu32(tokens, index * 12 + 4), buffer.readu32(tokens, index * 12 + 8)
  local line = lineOf(at)
  if blankBetween(lineOf(last), line) then return false end

  local text = string.sub(source, first, last)
  if string.find(text, '^%-%-!') or string.find(text, '^%-%-%-@') then return false end

  return string.find(text, '%w') ~= nil
end

-- The line facts the tokens give: which lines hold a token, which continue a long string or
-- comment, and where each line's first token starts.
local function scanLines()
  lineCount = #lineStarts
  lineFlags = buffer.create(lineCount)
  lineFirst = buffer.create(lineCount * 4)
  local line = 1

  for index = 0, tokenCount - 1 do
    local first, last =
      buffer.readu32(tokens, index * 12 + 4), buffer.readu32(tokens, index * 12 + 8)

    while line < lineCount and lineStarts[line + 1] <= first do
      line += 1
    end

    if buffer.readu8(lineFlags, line - 1) == 0 then
      buffer.writeu32(lineFirst, (line - 1) * 4, first)
    end

    buffer.writeu8(lineFlags, line - 1, bit32.bor(buffer.readu8(lineFlags, line - 1), HasToken))
    local lastLine = line

    while lastLine < lineCount and lineStarts[lastLine + 1] <= last do
      lastLine += 1
      buffer.writeu8(lineFlags, lastLine - 1, HasToken + ContinuesText)
    end
  end
end

local function checkIndentation()
  for line = 1, lineCount do
    local flags = buffer.readu8(lineFlags, line - 1)
    if flags == 0 or bit32.band(flags, ContinuesText) ~= 0 then continue end

    local start = lineStarts[line]
    local width = buffer.readu32(lineFirst, (line - 1) * 4) - start
    if width == 0 then continue end

    local tabbed = false

    for at = start, start + width - 1 do
      if string.byte(source, at) == 9 then
        tabbed = true
        break
      end
    end

    if tabbed then
      reportAt(
        'layout.indentation',
        line,
        1,
        width,
        'indented with a tab: indent with two spaces per level'
      )
    elseif width % 2 ~= 0 then
      reportAt(
        'layout.indentation',
        line,
        1,
        width,
        `indented by {width} spaces: indent in steps of two`
      )
    end
  end
end

local walkExpr: (expr: Expr) -> ()
local walkBlock: (block: Block) -> ()
local walkType: (node: Type) -> ()
local walkPack: (node: TypePack) -> ()

-- `name` as a full word: not one letter, not an abbreviation. `_` is the name for nothing.
local function checkWord(
  name: string,
  node: { line: number, column: number, endLine: number, endColumn: number }
)
  if #name == 1 and name ~= '_' then
    local hint = SingleLetters[string.lower(name)]
    local suffix = if hint then ` ({hint})` else ''
    report('naming.words', node, `'{name}' is a single letter: name what it holds{suffix}`)
    return
  end

  local spelled = Abbreviations[name]
  if spelled then
    report('naming.words', node, `'{name}' is an abbreviation: spell it out ({spelled})`)
  end
end

local function checkName(target: Local, isModule: boolean, isAlias: boolean)
  if not naming then return end

  local name = target.name

  if isPascal(name) then
    if isModule or (isAlias and not jessProfile) then
      table.insert(pascalLocals, target)
    else
      report(
        'naming.case',
        target,
        `'{name}' is PascalCase: only module constants are; locals are camelCase`
      )
    end
  elseif not isCamel(name) then
    local pascal = pascalCase(name)
    local camel = string.lower(string.sub(pascal, 1, 1)) .. string.sub(pascal, 2)
    report(
      'naming.case',
      target,
      `'{name}' is neither camelCase nor PascalCase: {camel}, or {pascal} for a module constant`
    )
  end

  checkWord(name, target)
end

local function declare(target: Local, isModule: boolean, isAlias: boolean?)
  checkName(target, isModule, isAlias == true)

  if isModule then moduleLocals[target] = true end

  if noAny and target.annotation then walkType(target.annotation) end
end

-- `string.format` for an index of a global library, `pcall` for a global; nil for anything else.
local function globalPath(expr: Expr): string?
  if expr.kind == 'ExprGlobal' then return expr.name end

  if expr.kind == 'ExprIndexName' and expr.op == '.' and expr.expr.kind == 'ExprGlobal' then
    return `{expr.expr.name}.{expr.index}`
  end

  return nil
end

local function isBuiltinAlias(expr: Expr): boolean
  if expr.kind == 'ExprGlobal' then return Builtins[expr.name] == true end

  local path = globalPath(expr)
  if not path or LibraryConstants[path] then return false end

  return expr.kind == 'ExprIndexName'
    and expr.expr.kind == 'ExprGlobal'
    and Builtins[expr.expr.name] == true
end

-- Whether a value is a literal: a constant, or a table of constants.
local function isLiteral(expr: Expr): boolean
  if
    expr.kind == 'ExprConstantString'
    or expr.kind == 'ExprConstantNumber'
    or expr.kind == 'ExprConstantBool'
  then
    return true
  end

  if expr.kind == 'ExprConstantInteger' then return true end

  if expr.kind == 'ExprUnary' then
    return expr.op == '-' and expr.expr.kind == 'ExprConstantNumber'
  end

  if expr.kind == 'ExprTable' then
    for _, item in expr.items do
      if item.key and not isLiteral(item.key) then return false end
      if not isLiteral(item.value) then return false end
    end

    return true
  end

  if expr.kind == 'ExprCall' and globalPath(expr.func) == 'table.freeze' and #expr.args == 1 then
    return isLiteral(expr.args[1])
  end

  return false
end

-- Type aliases declared at the top of the file, which a local's annotation may name.
local aliases: { [string]: Type } = {}

-- Whether a type may hold `false`: boolean itself, a type that could be anything (`any`, `unknown`,
-- a generic, a type from another module), or a union with one. An alias declared in the file is
-- followed; `seen` stops a recursive one.
local function admitsFalse(node: Type, seen: { [string]: boolean }): boolean
  if node.kind == 'TypeReference' then
    if node.prefix then return true end

    local name = node.name
    if ValueTypes[name] then return false end

    local alias = aliases[name]
    if not alias or seen[name] then return true end

    seen[name] = true
    return admitsFalse(alias, seen)
  end

  if node.kind == 'TypeSingletonBool' then return not node.value end

  if node.kind == 'TypeUnion' or node.kind == 'TypeIntersection' then
    for _, member in node.types do
      if admitsFalse(member, seen) then return true end
    end

    return false
  end

  if node.kind == 'TypeGroup' then return admitsFalse(node.type, seen) end

  return node.kind == 'TypeTypeof' or node.kind == 'TypeError'
end

-- Whether an expression can never be false, so `== nil` on it is a truthiness test spelled long.
local function neverFalse(expr: Expr): boolean
  if expr.kind == 'ExprLocal' then
    local target = expr['local']
    local annotation = target.annotation

    if annotation then return not admitsFalse(annotation, {}) end

    return constants[target] ~= nil or nonBoolean[target] == true
  end

  if expr.kind == 'ExprCall' then
    local path = globalPath(expr.func)
    return path ~= nil and NeverFalse[path] == true
  end

  return false
end

local function checkQuotes(expr: dream_luau_ExprConstantString)
  if expr.quoteStyle == 'double' and not string.find(expr.value, '\'', 1, true) then
    report(
      'quotes.double',
      expr,
      'single quotes: double quotes are for a string that holds a single quote'
    )
  elseif expr.quoteStyle == 'backtick' then
    report(
      'quotes.backtick',
      expr,
      'backticks are for interpolation: a string without any takes single quotes'
    )
  end
end

local function checkNilComparison(expr: dream_luau_ExprBinary)
  local other: Expr

  if expr.right.kind == 'ExprConstantNil' then
    other = expr.left
  elseif expr.left.kind == 'ExprConstantNil' then
    other = expr.right
  else
    return
  end

  if not neverFalse(other) then return end

  local spelled = if expr.op == '~=' then 'the value itself' else '`not` and the value'
  report('truthiness', expr, `this can't be false, so test {spelled} instead of comparing with nil`)
end

-- The comment goes right above the statement holding the call, or above a statement that holds
-- that one from its first line or the line under it: a function whose first line is the call, an
-- `if` whose body is.
local function checkProtectedCall(expr: Expr, name: string)
  if commentBefore(offset(expr.line, expr.column)) then return end

  protectedSeen = true
  local line = enclosingLines[depth]

  for level = depth, 1, -1 do
    local start = enclosingLines[level]
    if start < line - 1 then break end
    if commentBefore(offset(start, enclosingColumns[level])) then return end

    if runLines[level] > 0 and commentBefore(offset(runLines[level], runColumns[level])) then
      return
    end

    line = start
  end

  report(
    'pcall.comment',
    expr,
    `{string.upper(name)} WITHOUT A COMMENT. Every {name} gets a comment right above it naming the call that throws and why the script carries on. Write that comment, or take the {name} out and let it throw.`
  )
end

-- The type at the start of a LuaLS annotation's text: up to the first space outside brackets.
local function luaLsType(text: string): string
  local nesting = 0

  for index = 1, #text do
    local byte = string.byte(text, index)

    if byte == 60 or byte == 40 or byte == 91 or byte == 123 then
      nesting += 1
    elseif byte == 62 or byte == 41 or byte == 93 or byte == 125 then
      nesting -= 1
    elseif (byte == 32 or byte == 9) and nesting == 0 then
      return string.sub(text, 1, index - 1)
    end
  end

  return text
end

-- Whether every member of a LuaLS union is a built-in type that is never false: a string, a
-- number, a table, an array, or a function. A class or alias name may be anything.
local function luaLsNeverFalse(text: string): boolean
  local depth, start = 0, 1
  local members: { string } = {}

  for index = 1, #text do
    local byte = string.byte(text, index)

    if byte == 60 or byte == 40 or byte == 91 or byte == 123 then
      depth += 1
    elseif byte == 62 or byte == 41 or byte == 93 or byte == 125 then
      depth -= 1
    elseif byte == 124 and depth == 0 then
      table.insert(members, string.sub(text, start, index - 1))
      start = index + 1
    end
  end

  table.insert(members, string.sub(text, start))

  for _, member in members do
    local bare = string.gsub(member, '%?$', '')
    if LuaLsValueTypes[bare] or string.find(bare, '%[%]$') then continue end
    if string.find(bare, '^table<') or string.find(bare, '^fun%(') then continue end

    return false
  end

  return true
end

-- LuaLS `---@param name type` lines in the comments right above the line a function starts on, by
-- parameter name: in Lua, the only types there are.
local function documentedParameters(line: number): { [string]: string }?
  local first = buffer.readu32(lineFirst, (line - 1) * 4)
  local index = tokenAt(first) - 1
  local found: { [string]: string }? = nil
  local below = line

  while index >= 0 do
    local kind = buffer.readu32(tokens, index * 12)
    if kind ~= CommentKind then break end

    local start, last =
      buffer.readu32(tokens, index * 12 + 4), buffer.readu32(tokens, index * 12 + 8)
    local commentLine = lineOf(start)
    if commentLine ~= below - 1 then break end

    local name, rest =
      string.match(string.sub(source, start, last), '^%-%-%-@param%s+([%w_]+)%??%s+(.*)$')

    if name and rest then
      found = found or {}
      found[name] = luaLsType(rest)
    end

    below, index = commentLine, index - 1
  end

  return found
end

local function walkFunction(func: Function)
  local documented = if jessProfile then nil else documentedParameters(func.line)

  for _, argument in func.args do
    declare(argument, false)
    local documentedType = documented and documented[argument.name]

    if documentedType and luaLsNeverFalse(documentedType) then nonBoolean[argument] = true end
  end

  if noAny then
    if func.varargAnnotation then walkPack(func.varargAnnotation) end
    if func.returnAnnotation then walkPack(func.returnAnnotation) end
  end

  walkBlock(func.body)
end

-- The local a reassignment or a mutation reaches through: `x`, `x.y`, `x[y]`, `x.y.z`.
local function rootLocal(expr: Expr): Local?
  while expr.kind == 'ExprIndexName' or expr.kind == 'ExprIndexExpr' do
    expr = expr.expr
  end

  if expr.kind == 'ExprLocal' then return expr['local'] end

  return nil
end

walkExpr = function(expr: Expr)
  local test = testing
  testing = false

  if expr.kind == 'ExprLocal' or expr.kind == 'ExprConstantNumber' then
    return
  elseif expr.kind == 'ExprConstantString' then
    checkQuotes(expr)
  elseif expr.kind == 'ExprGlobal' then
    local name = expr.name
    if name == 'pcall' or name == 'xpcall' then checkProtectedCall(expr, name) end
  elseif expr.kind == 'ExprIndexName' then
    walkExpr(expr.expr)
  elseif expr.kind == 'ExprCall' then
    walkExpr(expr.func)

    for _, argument in expr.args do
      walkExpr(argument)

      if argument.kind == 'ExprLocal' then mutated[argument['local']] = true end
    end

    if expr.self and expr.func.kind == 'ExprIndexName' then
      local target = rootLocal(expr.func.expr)
      if target then mutated[target] = true end
    end

    local path = globalPath(expr.func)

    if path and Mutators[path] and expr.args[1] then
      local target = rootLocal(expr.args[1])
      if target then mutated[target] = true end
    end
  elseif expr.kind == 'ExprBinary' then
    local logical = expr.op == 'and' or expr.op == 'or'
    testing = test and logical
    walkExpr(expr.left)
    testing = test and logical
    walkExpr(expr.right)

    if test and (expr.op == '==' or expr.op == '~=') then checkNilComparison(expr) end
  elseif expr.kind == 'ExprFunction' then
    if expr.endLine > expr.line then nestedBlock = true end

    walkFunction(expr)
  elseif expr.kind == 'ExprTable' then
    if expr.endLine > expr.line then nestedBlock = true end

    for _, item in expr.items do
      local key = item.key

      if key and item.itemKind == 'record' and naming and key.kind == 'ExprConstantString' then
        checkWord(key.value, key)
      end

      if key then walkExpr(key) end

      walkExpr(item.value)
    end
  elseif expr.kind == 'ExprIndexExpr' then
    walkExpr(expr.expr)
    walkExpr(expr.index)
  elseif expr.kind == 'ExprUnary' then
    testing = test and expr.op == 'not'
    walkExpr(expr.expr)
  elseif expr.kind == 'ExprGroup' then
    testing = test
    walkExpr(expr.expr)
  elseif expr.kind == 'ExprInterpString' then
    for _, part in expr.expressions do
      walkExpr(part)
    end
  elseif expr.kind == 'ExprIfElse' then
    testing = true
    walkExpr(expr.condition)
    walkExpr(expr.trueExpr)
    walkExpr(expr.falseExpr)
  elseif expr.kind == 'ExprTypeAssertion' then
    walkExpr(expr.expr)

    if noAny then walkType(expr.annotation) end
  elseif expr.kind == 'ExprInstantiate' then
    walkExpr(expr.expr)
  elseif expr.kind == 'ExprError' then
    for _, part in expr.expressions do
      walkExpr(part)
    end
  end
end

walkType = function(node: Type)
  if node.kind == 'TypeReference' then
    if node.name == 'any' and not node.prefix then
      report('types.any', node, '`any` turns the type checker off here: name the real type')
    end

    for _, parameter in node.parameters do
      if parameter.kind == 'TypePackExplicit' or parameter.kind == 'TypePackVariadic' then
        walkPack(parameter)
      elseif parameter.kind ~= 'TypePackGeneric' then
        walkType(parameter)
      end
    end
  elseif node.kind == 'TypeTable' then
    for _, property in node.props do
      walkType(property.type)
    end

    if node.indexer then
      walkType(node.indexer.indexType)
      walkType(node.indexer.resultType)
    end
  elseif node.kind == 'TypeFunction' then
    for _, argument in node.argTypes.types do
      walkType(argument)
    end

    if node.argTypes.tailType then walkPack(node.argTypes.tailType) end

    walkPack(node.returnTypes)
  elseif node.kind == 'TypeUnion' or node.kind == 'TypeIntersection' then
    for _, member in node.types do
      walkType(member)
    end
  elseif node.kind == 'TypeGroup' then
    walkType(node.type)
  end
end

walkPack = function(node: TypePack)
  if node.kind == 'TypePackExplicit' then
    for _, member in node.typeList.types do
      walkType(member)
    end

    if node.typeList.tailType then walkPack(node.typeList.tailType) end
  elseif node.kind == 'TypePackVariadic' then
    walkType(node.variadicType)
  end
end

-- Whether a statement is compound by its kind alone: it has a body.
local function hasBody(stat: Stat): boolean
  local kind = stat.kind
  return kind == 'StatIf'
    or kind == 'StatFor'
    or kind == 'StatForIn'
    or kind == 'StatWhile'
    or kind == 'StatRepeat'
    or kind == 'StatBlock'
    or kind == 'StatFunction'
    or kind == 'StatLocalFunction'
end

-- Whether a block ends by leaving it: return, break, continue, or error.
local function leaves(block: Block): boolean
  local last = block.body[#block.body]
  if not last then return false end

  if last.kind == 'StatReturn' or last.kind == 'StatBreak' or last.kind == 'StatContinue' then
    return true
  end

  return last.kind == 'StatExpr'
    and last.expr.kind == 'ExprCall'
    and last.expr.func.kind == 'ExprGlobal'
    and last.expr.func.name == 'error'
end

local function checkIf(stat: dream_luau_StatIf)
  if not guards then return end

  local elsebody = stat.elsebody
  if not elsebody or elsebody.kind ~= 'StatBlock' or not leaves(stat.thenbody) then return end
  if #elsebody.body < 2 and not (elsebody.body[1] and hasBody(elsebody.body[1])) then return end

  report(
    'guard.else',
    elsebody,
    'the branch above leaves, so this else only adds a level: end the if there and let this run after it'
  )
end

-- A function or loop body whose whole work hangs off one condition: invert it and leave early.
local function checkWrapped(body: Block, inLoop: boolean)
  if not guards then return end

  local last = body.body[#body.body]
  if not last or last.kind ~= 'StatIf' or last.elsebody then return end

  local inner = last.thenbody.body
  if #inner < 3 then return end

  for index = 1, #body.body - 1 do
    if body.body[index].kind ~= 'StatLocal' then return end
  end

  local exit = if inLoop then 'continue' else 'return'
  report(
    'guard.nested',
    last,
    `the rest of the body hangs off this condition: test the opposite, {exit} early, and keep the work at the outer level`
  )
end

-- A local's annotation as written, or '' without one: locals of different types aren't the same kind.
local function annotation(target: Local): string
  local node = target.annotation
  if not node then return '' end

  return string.sub(source, offset(node.line, node.column), offset(node.endLine, node.endColumn))
end

local function checkDeclarations(stat: dream_luau_StatLocal, previous: Stat?, separated: boolean)
  local vars, values = stat.vars, stat.values

  if jessProfile then
    for index, target in vars do
      local value = values[index]

      if value and isBuiltinAlias(value) then
        report(
          'hoisting',
          target,
          `'{target.name}' only renames {globalPath(value) or 'a global'}: use it where it's needed, Luau doesn't need the alias`
        )
      end
    end
  end

  local lastValue = values[#values]
  local open = lastValue and (lastValue.kind == 'ExprCall' or lastValue.kind == 'ExprVarargs')

  if #values > 0 and #values < #vars and not open then
    report(
      'locals.mixed',
      stat,
      'some of these locals start nil: declare the uninitialized ones on a line of their own'
    )
  end

  if not previous or separated or previous.kind ~= 'StatLocal' then return end
  if stat.line ~= stat.endLine or previous.line ~= previous.endLine then return end

  local before = previous :: dream_luau_StatLocal
  if #before.vars ~= 1 or #vars ~= 1 or #before.values ~= #values or #values > 1 then return end
  if annotation(before.vars[1]) ~= annotation(vars[1]) then return end

  local shape: string? = nil

  if #values == 1 then
    local left, right = before.values[1], values[1]

    if
      left.kind == 'ExprTable'
      and right.kind == 'ExprTable'
      and #left.items == 0
      and #right.items == 0
    then
      shape = 'empty tables'
    end
  end

  if not shape then return end

  report(
    'locals.group',
    stat,
    `'{before.vars[1].name}' and '{vars[1].name}' are both {shape}: declare them on one line`
  )
end

local function walkStat(stat: Stat, isModule: boolean)
  if stat.kind == 'StatExpr' then
    walkExpr(stat.expr)
  elseif stat.kind == 'StatLocal' then
    for _, value in stat.values do
      walkExpr(value)
    end

    for index, target in stat.vars do
      local value = stat.values[index]
      local isAlias = value ~= nil and (value.kind == 'ExprIndexName' or value.kind == 'ExprGlobal')
      declare(target, isModule, isAlias)

      if value and isLiteral(value) then
        constants[target] = if value.kind == 'ExprTable' then 'table' else 'value'
      end
    end
  elseif stat.kind == 'StatAssign' then
    for _, target in stat.vars do
      if target.kind == 'ExprLocal' then
        reassigned[target['local']] = true
      else
        walkExpr(target)
        local root = rootLocal(target)
        if root then mutated[root] = true end
      end
    end

    for _, value in stat.values do
      walkExpr(value)
    end
  elseif stat.kind == 'StatCompoundAssign' then
    if stat.var.kind == 'ExprLocal' then
      reassigned[stat.var['local']] = true
    else
      walkExpr(stat.var)
      local root = rootLocal(stat.var)
      if root then mutated[root] = true end
    end

    walkExpr(stat.value)
  elseif stat.kind == 'StatIf' then
    testing = true
    walkExpr(stat.condition)
    walkBlock(stat.thenbody)

    if stat.elsebody then walkStat(stat.elsebody, false) end

    checkIf(stat)
  elseif stat.kind == 'StatReturn' then
    for _, value in stat.list do
      walkExpr(value)
    end
  elseif stat.kind == 'StatLocalFunction' then
    declare(stat.name, false)
    walkFunction(stat.func)
    checkWrapped(stat.func.body, false)
  elseif stat.kind == 'StatFunction' then
    local root = rootLocal(stat.name)

    if root and stat.name.kind ~= 'ExprLocal' then
      mutated[root] = true
    elseif root then
      reassigned[root] = true
    end

    walkFunction(stat.func)
    checkWrapped(stat.func.body, false)
  elseif stat.kind == 'StatForIn' then
    for _, value in stat.values do
      walkExpr(value)
    end

    for _, target in stat.vars do
      declare(target, false)
    end

    walkBlock(stat.body)
    checkWrapped(stat.body, true)
  elseif stat.kind == 'StatFor' then
    walkExpr(stat.from)
    walkExpr(stat.to)

    if stat.step then walkExpr(stat.step) end

    declare(stat.var, false)
    walkBlock(stat.body)
    checkWrapped(stat.body, true)
  elseif stat.kind == 'StatWhile' then
    testing = true
    walkExpr(stat.condition)
    walkBlock(stat.body)
    checkWrapped(stat.body, true)
  elseif stat.kind == 'StatRepeat' then
    walkBlock(stat.body)
    testing = true
    walkExpr(stat.condition)
  elseif stat.kind == 'StatBlock' then
    walkBlock(stat)
  elseif stat.kind == 'StatTypeAlias' then
    aliases[stat.name] = stat.type

    if noAny then walkType(stat.type) end
  elseif stat.kind == 'StatTypeFunction' then
    walkFunction(stat.body)
  elseif stat.kind == 'StatError' then
    for _, part in stat.expressions do
      walkExpr(part)
    end

    for _, inner in stat.statements do
      walkStat(inner, false)
    end
  elseif stat.kind == 'StatDeclareGlobal' then
    if noAny then walkType(stat.type) end
  elseif stat.kind == 'StatDeclareFunction' then
    if noAny then
      for _, argument in stat.params.types do
        walkType(argument)
      end

      walkPack(stat.retTypes)
    end
  elseif stat.kind == 'StatDeclareExternType' then
    if noAny then
      for _, property in stat.props do
        walkType(property.type)
      end
    end
  end
end

walkBlock = function(block: Block)
  local isModule = depth == 0 or block == moduleBody
  local previous: Stat? = nil
  local previousCompound, previousMultiLine, previousProtected = false, false, false
  local runLine, runColumn = 0, 0

  for _, stat in block.body do
    local separated = previous ~= nil and blankBetween(previous.endLine, stat.line)

    if previousProtected and not separated and previous then
      if runLine == 0 then
        runLine, runColumn = previous.line, previous.column
      end
    else
      runLine, runColumn = 0, 0
    end

    local outer, outerProtected = nestedBlock, protectedSeen
    depth += 1
    enclosingLines[depth], enclosingColumns[depth] = stat.line, stat.column
    runLines[depth], runColumns[depth] = runLine, runColumn
    nestedBlock, protectedSeen = false, false
    walkStat(stat, isModule)

    local compound = nestedBlock or hasBody(stat)
    local multiLine = compound and stat.endLine > stat.line

    if
      previous
      and not separated
      and compound
      and previousCompound
      and (multiLine or previousMultiLine)
    then
      report(
        'layout.compound',
        stat,
        'needs a blank line between this compound statement and the one before it: a multi-line one always gets room'
      )
    end

    if stat.kind == 'StatLocal' then checkDeclarations(stat, previous, separated) end

    depth -= 1
    previousProtected = protectedSeen
    nestedBlock, protectedSeen = outer, outerProtected or protectedSeen
    previous, previousCompound, previousMultiLine = stat, compound, multiLine
  end
end

-- PascalCase is for module constants; a module-level local that is a literal and is never changed
-- is one, and is PascalCase.
local function checkConstants()
  if not naming then return end

  for _, target in pascalLocals do
    if reassigned[target] and jessProfile then
      report(
        'naming.case',
        target,
        `'{target.name}' is reassigned, so it isn't a constant: name it in camelCase`
      )
    end
  end

  if options.fragment then return end

  for target in moduleLocals do
    local constant = constants[target]
    if not constant or reassigned[target] then continue end
    if constant == 'table' and mutated[target] then continue end
    if not isCamel(target.name) then continue end

    report(
      'naming.constant',
      target,
      `'{target.name}' is a module constant: name it {pascalCase(target.name)}`
    )
  end
end

local function checkDirectives(result: dream_luau_ParseResult)
  local required = options.directives
  if not required then return end

  local present: { [string]: boolean } = {}

  for _, comment in result.hotComments do
    if comment.header then present[comment.content] = true end
  end

  for _, name in required do
    if present[name] then continue end

    reportAt(`directives.{name}`, 1, 1, 1, `this file starts with --!{name}`)
  end
end

local function compare(left: Finding, right: Finding): boolean
  if left.line ~= right.line then return left.line < right.line end
  if left.column ~= right.column then return left.column < right.column end

  return left.rule < right.rule
end

-- Every finding in `text`, sorted by position.
local function check(text: string, given: Options?): { Finding }
  options = given or ({} :: Options)
  local profile = options.profile or 'jess'
  jessProfile, only = profile == 'jess', nil

  if options.only then
    local selected: { [string]: boolean } = {}

    for _, rule in options.only do
      if not rules.Rules[rule] then error(`@jess/style: there is no rule '{rule}'`) end

      selected[rule] = true
    end

    only = selected
  end

  local function wanted(rule: string): boolean return not only or only[rule] == true end

  naming = wanted 'naming.case' or wanted 'naming.constant' or wanted 'naming.words'
  guards = wanted 'guard.else' or wanted 'guard.nested'
  noAny = options.noAny == true
  source, findings = text, {}
  reassigned, mutated, constants, nonBoolean, moduleLocals, pascalLocals, aliases =
    {}, {}, {}, {}, {}, {}, {}

  local result = luau.parse(text, { tokens = true, declarations = options.declarations })

  -- Without a say either way, a source that only parses as definitions is a definitions file.
  if #result.errors > 0 and options.declarations == nil then
    local declared = luau.parse(text, { tokens = true, declarations = true })
    if #declared.errors == 0 then result = declared end
  end

  lineStarts, tokens = result.lineStarts, result.tokens :: buffer
  tokenCount = buffer.len(tokens) // 12

  for _, problem in result.errors do
    report('syntax', problem, problem.message)
  end

  scanLines()
  checkIndentation()
  checkDirectives(result)
  depth, nestedBlock, protectedSeen = 0, false, false
  local root = result.root

  for _, stat in root.body do
    if stat.kind == 'StatTypeAlias' then aliases[stat.name] = stat.type end
  end

  local last = root.body[#root.body]
  moduleBody = nil

  if last and last.kind == 'StatReturn' and #last.list == 1 then
    local returned = last.list[1]
    if returned.kind == 'ExprFunction' then moduleBody = returned.body end
  end

  walkBlock(root)
  checkConstants()
  table.sort(findings, compare)

  local found = findings
  source, findings, tokens, lineFlags, lineFirst =
    '', {}, buffer.create(0), buffer.create(0), buffer.create(0)
  reassigned, mutated, constants, nonBoolean, moduleLocals, pascalLocals, aliases =
    {}, {}, {}, {}, {}, {}, {}
  return found
end

return check
