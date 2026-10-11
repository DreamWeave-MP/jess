--!strict
--!native
-- @jess/analysis: reusable semantic facts over Luau source for Jess scripts and packages.
--
-- This module deliberately knows Luau and Jess, not OpenMW. Domain packages consume its stable
-- source/workspace facts and add their own semantics without growing builtin.jpk into a plugin
-- framework.
local luau = require '@dream/luau'

export type Span = {
  line: number,
  column: number,
  endLine: number,
  endColumn: number,
}

export type Origin = {
  kind: 'global' | 'module',
  name: string,
  member: string?,
}

export type RequireFact = Span & {
  module: string,
  binding: dream_luau_Local?,
  protected: boolean?,
  via: string?,
  functionDepth: number,
  loopDepth: number,
  conditionalDepth: number,
  expr: dream_luau_ExprCall,
}

export type CallFact = Span & {
  target: string?,
  module: string?,
  member: string?,
  functionDepth: number,
  loopDepth: number,
  conditionalDepth: number,
  expr: dream_luau_ExprCall,
}

export type FunctionFact = Span & {
  name: string?,
  functionDepth: number,
  loopDepth: number,
  conditionalDepth: number,
  node: dream_luau_ExprFunction,
}

export type MemberFact = Span & {
  target: string?,
  module: string?,
  member: string?,
  functionDepth: number,
  loopDepth: number,
  conditionalDepth: number,
  expr: dream_luau_ExprIndexName,
}

export type TableFact = Span & {
  functionDepth: number,
  loopDepth: number,
  conditionalDepth: number,
  expr: dream_luau_ExprTable,
}

export type AssignmentFact = Span & {
  target: string?,
  functionDepth: number,
  loopDepth: number,
  conditionalDepth: number,
  expr: dream_luau_Expr,
  value: dream_luau_Expr?,
}

export type Source = {
  path: string?,
  source: string,
  root: dream_luau_StatBlock,
  errors: { dream_luau_ParseError },
  comments: { dream_luau_Comment },
  hotComments: { dream_luau_HotComment },
  lineStarts: { number },
  requires: { RequireFact },
  calls: { CallFact },
  functions: { FunctionFact },
  members: { MemberFact },
  tables: { TableFact },
  assignments: { AssignmentFact },
  globals: { [string]: number },
  strings: { { value: string, line: number, column: number } },
  origins: { [dream_luau_Local]: Origin },
  localValues: { [dream_luau_Local]: dream_luau_Expr },
  moduleReturn: dream_luau_Expr?,
}

export type Workspace = {
  roots: { string },
  files: { string },
  sources: { Source },
  byPath: { [string]: Source },
  byModule: { [string]: Source },
  requireEdges: { { from: string, to: string, module: string, line: number } },
  unresolvedRequires: { { from: string, module: string, line: number } },
}

export type ParseOptions = {
  declarations: boolean?,
  path: string?,
}

export type WorkspaceOptions = {
  extensions: { string }?,
  skipDirectories: { string }?,
  transform: ((source: string, path: string) -> string)?,
}

type Expr = dream_luau_Expr
type Stat = dream_luau_Stat
type Block = dream_luau_StatBlock
type Local = dream_luau_Local

local function withSpan<Value>(value: Value, node: Span): Value & Span
  local into = (value :: unknown) :: Value & Span
  into.line = node.line
  into.column = node.column
  into.endLine = node.endLine
  into.endColumn = node.endColumn
  return into
end

local function literalString(expr: Expr): string?
  if expr.kind == 'ExprConstantString' then return expr.value end
  return nil
end

local function rawPath(expr: Expr): string?
  if expr.kind == 'ExprGlobal' then return expr.name end

  if expr.kind == 'ExprIndexName' then
    local base = rawPath(expr.expr)
    if base then return `{base}.{expr.index}` end
  end

  return nil
end

local function originPath(origin: Origin): (string, string?, string?)
  if origin.kind == 'module' then
    if origin.member then return `{origin.name}.{origin.member}`, origin.name, origin.member end
    return origin.name, origin.name, nil
  end

  return origin.name, nil, nil
end

local function qualifiedPath(expr: Expr, origins: { [Local]: Origin }): (string?, string?, string?)
  if expr.kind == 'ExprGlobal' then return expr.name, nil, nil end

  if expr.kind == 'ExprCall' and rawPath(expr.func) == 'require' and #expr.args == 1 then
    local moduleName = literalString(expr.args[1])
    if moduleName then return moduleName, moduleName, nil end
  end

  if expr.kind == 'ExprLocal' then
    local origin = origins[expr['local']]
    if origin then return originPath(origin) end
    return expr['local'].name, nil, nil
  end

  if expr.kind == 'ExprIndexName' then
    if expr.expr.kind == 'ExprLocal' then
      local origin = origins[expr.expr['local']]
      if origin and origin.kind == 'module' then
        local member = if origin.member then `{origin.member}.{expr.index}` else expr.index
        return `{origin.name}.{member}`, origin.name, member
      end
    end

    local base, module, member = qualifiedPath(expr.expr, origins)
    if not base then return nil, nil, nil end

    if module then
      local joined = if member then `{member}.{expr.index}` else expr.index
      return `{module}.{joined}`, module, joined
    end

    return `{base}.{expr.index}`, nil, nil
  end

  if expr.kind == 'ExprGroup' or expr.kind == 'ExprTypeAssertion' or expr.kind == 'ExprInstantiate' then
    return qualifiedPath(expr.expr, origins)
  end

  return nil, nil, nil
end

local function expressionOrigin(expr: Expr, origins: { [Local]: Origin }): Origin?
  if expr.kind == 'ExprGlobal' then return { kind = 'global', name = expr.name } end

  if expr.kind == 'ExprLocal' then return origins[expr['local']] end

  if expr.kind == 'ExprCall' then
    local target = rawPath(expr.func)
    if target == 'require' and #expr.args == 1 then
      local moduleName = literalString(expr.args[1])
      if moduleName then return { kind = 'module', name = moduleName } end
    end

    return nil
  end

  if expr.kind == 'ExprIndexName' then
    local origin = expressionOrigin(expr.expr, origins)
    if not origin then return nil end

    if origin.kind == 'module' then
      local member = if origin.member then `{origin.member}.{expr.index}` else expr.index
      return { kind = 'module', name = origin.name, member = member }
    end

    return { kind = 'global', name = `{origin.name}.{expr.index}` }
  end

  if expr.kind == 'ExprGroup' or expr.kind == 'ExprTypeAssertion' or expr.kind == 'ExprInstantiate' then
    return expressionOrigin(expr.expr, origins)
  end

  return nil
end

local function analyze(source: string, options: ParseOptions?): Source
  local given: ParseOptions = options or {}
  local result = luau.parse(source, { declarations = given.declarations })

  if #result.errors > 0 and given.declarations == nil then
    local declared = luau.parse(source, { declarations = true })
    if #declared.errors == 0 then result = declared end
  end

  local origins: { [Local]: Origin } = {}
  local localValues: { [Local]: Expr } = {}
  local requires: { RequireFact } = {}
  local calls: { CallFact } = {}
  local functions: { FunctionFact } = {}
  local members: { MemberFact } = {}
  local tables: { TableFact } = {}
  local assignments: { AssignmentFact } = {}
  local globals: { [string]: number } = {}
  local strings = {}
  local functionDepth, loopDepth, conditionalDepth = 0, 0, 0

  local walkExpr: (Expr) -> ()
  local walkStat: (Stat) -> ()
  local walkBlock: (Block) -> ()

  local function rememberCall(expr: dream_luau_ExprCall)
    local target, module, member = qualifiedPath(expr.func, origins)
    local fact: CallFact = withSpan({
      target = target,
      module = module,
      member = member,
      functionDepth = functionDepth,
      loopDepth = loopDepth,
      conditionalDepth = conditionalDepth,
      expr = expr,
    }, expr)
    table.insert(calls, fact)

    if target == 'require' and #expr.args == 1 then
      local moduleName = literalString(expr.args[1])
      if moduleName then
        local requireFact: RequireFact = withSpan({
          module = moduleName,
          binding = nil :: Local?,
          protected = false :: boolean?,
          via = nil :: string?,
          functionDepth = functionDepth,
          loopDepth = loopDepth,
          conditionalDepth = conditionalDepth,
          expr = expr,
        }, expr)
        table.insert(requires, requireFact)
      end
    elseif target == 'pcall' and #expr.args >= 2 and rawPath(expr.args[1]) == 'require' then
      local moduleName = literalString(expr.args[2])
      if moduleName then
        local requireFact: RequireFact = withSpan({
          module = moduleName,
          binding = nil :: Local?,
          protected = true :: boolean?,
          via = 'pcall' :: string?,
          functionDepth = functionDepth,
          loopDepth = loopDepth,
          conditionalDepth = conditionalDepth,
          expr = expr,
        }, expr)
        table.insert(requires, requireFact)
      end
    end
  end

  local function walkFunction(expr: dream_luau_ExprFunction, name: string?)
    local fact: FunctionFact = withSpan({
      name = name or expr.debugname,
      functionDepth = functionDepth,
      loopDepth = loopDepth,
      conditionalDepth = conditionalDepth,
      node = expr,
    }, expr)
    table.insert(functions, fact)

    functionDepth += 1
    walkBlock(expr.body)
    functionDepth -= 1
  end

  walkExpr = function(expr: Expr)
    if expr.kind == 'ExprGlobal' then
      globals[expr.name] = (globals[expr.name] or 0) + 1
      return
    end

    if expr.kind == 'ExprConstantString' then
      table.insert(strings, { value = expr.value, line = expr.line, column = expr.column })
      return
    end

    if expr.kind == 'ExprLocal' or expr.kind == 'ExprConstantNil' or expr.kind == 'ExprConstantBool'
      or expr.kind == 'ExprConstantNumber' or expr.kind == 'ExprConstantInteger' or expr.kind == 'ExprVarargs'
    then
      return
    end

    if expr.kind == 'ExprGroup' or expr.kind == 'ExprUnary' or expr.kind == 'ExprTypeAssertion' or expr.kind == 'ExprInstantiate' then
      walkExpr(expr.expr)
      return
    end

    if expr.kind == 'ExprIndexName' then
      local target, module, member = qualifiedPath(expr, origins)
      local fact: MemberFact = withSpan({
        target = target,
        module = module,
        member = member,
        functionDepth = functionDepth,
        loopDepth = loopDepth,
        conditionalDepth = conditionalDepth,
        expr = expr,
      }, expr)
      table.insert(members, fact)
      walkExpr(expr.expr)
      return
    end

    if expr.kind == 'ExprIndexExpr' then
      walkExpr(expr.expr)
      walkExpr(expr.index)
      return
    end

    if expr.kind == 'ExprCall' then
      rememberCall(expr)
      walkExpr(expr.func)
      for _, argument in expr.args do walkExpr(argument) end
      return
    end

    if expr.kind == 'ExprFunction' then
      walkFunction(expr, nil)
      return
    end

    if expr.kind == 'ExprTable' then
      local fact: TableFact = withSpan({ functionDepth = functionDepth, loopDepth = loopDepth, conditionalDepth = conditionalDepth, expr = expr }, expr)
      table.insert(tables, fact)
      for _, item in expr.items do
        if item.key then walkExpr(item.key) end
        walkExpr(item.value)
      end
      return
    end

    if expr.kind == 'ExprBinary' then
      walkExpr(expr.left)
      walkExpr(expr.right)
      return
    end

    if expr.kind == 'ExprIfElse' then
      walkExpr(expr.condition)
      conditionalDepth += 1
      walkExpr(expr.trueExpr)
      walkExpr(expr.falseExpr)
      conditionalDepth -= 1
      return
    end

    if expr.kind == 'ExprInterpString' then
      for _, value in expr.expressions do walkExpr(value) end
      return
    end

    if expr.kind == 'ExprError' then
      for _, value in expr.expressions do walkExpr(value) end
    end
  end

  local function rememberAssignment(expr: Expr, value: Expr?)
    local target = qualifiedPath(expr, origins)
    local fact: AssignmentFact = withSpan({
      target = target,
      functionDepth = functionDepth,
      loopDepth = loopDepth,
      conditionalDepth = conditionalDepth,
      expr = expr,
      value = value,
    }, expr)
    table.insert(assignments, fact)
  end

  walkStat = function(stat: Stat)
    if stat.kind == 'StatBlock' then
      walkBlock(stat)
      return
    end

    if stat.kind == 'StatIf' then
      walkExpr(stat.condition)
      conditionalDepth += 1
      walkBlock(stat.thenbody)
      if stat.elsebody then walkStat(stat.elsebody) end
      conditionalDepth -= 1
      return
    end

    if stat.kind == 'StatWhile' then
      walkExpr(stat.condition)
      loopDepth += 1
      walkBlock(stat.body)
      loopDepth -= 1
      return
    end

    if stat.kind == 'StatRepeat' then
      loopDepth += 1
      walkBlock(stat.body)
      loopDepth -= 1
      walkExpr(stat.condition)
      return
    end

    if stat.kind == 'StatReturn' then
      for _, value in stat.list do walkExpr(value) end
      return
    end

    if stat.kind == 'StatExpr' then
      walkExpr(stat.expr)
      return
    end

    if stat.kind == 'StatLocal' then
      for _, value in stat.values do walkExpr(value) end

      for index, target in stat.vars do
        local value = stat.values[index]
        if not value then continue end

        localValues[target] = value
        local origin = expressionOrigin(value, origins)
        if origin then
          origins[target] = origin
        end

        if value.kind == 'ExprCall' and rawPath(value.func) == 'require' and #value.args == 1 then
          local moduleName = literalString(value.args[1])
          if moduleName then
            for scan = #requires, 1, -1 do
              local found = requires[scan]
              if found.expr == value then
                found.binding = target
                break
              end
            end
          end
        end
      end

      return
    end

    if stat.kind == 'StatFor' then
      walkExpr(stat.from)
      walkExpr(stat.to)
      if stat.step then walkExpr(stat.step) end
      loopDepth += 1
      walkBlock(stat.body)
      loopDepth -= 1
      return
    end

    if stat.kind == 'StatForIn' then
      for _, value in stat.values do walkExpr(value) end
      loopDepth += 1
      walkBlock(stat.body)
      loopDepth -= 1
      return
    end

    if stat.kind == 'StatAssign' then
      for index, target in stat.vars do
        rememberAssignment(target, stat.values[index])
        walkExpr(target)
      end

      -- Lua evaluates every RHS before mutating any LHS. Preserve that ordering so
      -- qualified paths observed while walking the values still see the old aliases.
      for _, value in stat.values do walkExpr(value) end

      -- Straight-line local reassignment is useful semantic provenance without
      -- requiring control-flow analysis. Guarded/loop assignments intentionally do
      -- not update the stable origin map: after a branch we cannot prove which value
      -- won, so consumers should see an unresolved local rather than a false alias.
      if conditionalDepth == 0 and loopDepth == 0 then
        for index, target in stat.vars do
          if target.kind ~= 'ExprLocal' then continue end
          local value = stat.values[index]
          if not value then continue end
          local localValue = target['local']
          localValues[localValue] = value
          local origin = expressionOrigin(value, origins)
          if origin then origins[localValue] = origin else origins[localValue] = nil end
        end
      end
      return
    end

    if stat.kind == 'StatCompoundAssign' then
      rememberAssignment(stat.var, stat.value)
      walkExpr(stat.var)
      walkExpr(stat.value)
      return
    end

    if stat.kind == 'StatFunction' then
      walkExpr(stat.name)
      local name = qualifiedPath(stat.name, origins)
      walkFunction(stat.func, name)
      return
    end

    if stat.kind == 'StatLocalFunction' then
      localValues[stat.name] = stat.func
      walkFunction(stat.func, stat.name.name)
      return
    end

    if stat.kind == 'StatTypeFunction' then
      walkFunction(stat.body, stat.name)
      return
    end

    if stat.kind == 'StatClass' then
      if stat.super then walkExpr(stat.super) end
      return
    end

    if stat.kind == 'StatError' then
      for _, value in stat.expressions do walkExpr(value) end
      for _, child in stat.statements do walkStat(child) end
    end
  end

  walkBlock = function(block: Block)
    for _, stat in block.body do walkStat(stat) end
  end

  -- Luau can return a nil root for sufficiently malformed/unsupported source. Keep parse
  -- diagnostics as first-class facts and analyze an empty synthetic block instead of crashing the
  -- entire workspace. This is especially important when a workspace is used as a corpus/indexer.
  local root = result.root
  if not root then
    local empty = luau.parse('')
    root = empty.root
  end

  walkBlock(root)

  local moduleReturn: Expr? = nil
  local last = root.body[#root.body]
  if last and last.kind == 'StatReturn' and #last.list == 1 then moduleReturn = last.list[1] end

  return {
    path = given.path,
    source = source,
    root = root,
    errors = result.errors,
    comments = result.comments,
    hotComments = result.hotComments,
    lineStarts = result.lineStarts,
    requires = requires,
    calls = calls,
    functions = functions,
    members = members,
    tables = tables,
    assignments = assignments,
    globals = globals,
    strings = strings,
    origins = origins,
    localValues = localValues,
    moduleReturn = moduleReturn,
  }
end

local function normalizePath(path: string): string
  local normalized = string.gsub(path, '\\', '/')
  normalized = string.gsub(normalized, '^%./', '')
  normalized = string.gsub(normalized, '/+', '/')
  return normalized
end

local function defaultExtensions(): { [string]: boolean }
  return { ['.lua'] = true, ['.luau'] = true }
end

local function extension(path: string): string
  return string.lower(string.match(path, '(%.[^./]+)$') or '')
end

local function collectFiles(paths: { string }, options: WorkspaceOptions?): { string }
  local given: WorkspaceOptions = options or {}
  local extensions = defaultExtensions()
  if given.extensions then
    extensions = {}
    for _, value in given.extensions do extensions[string.lower(value)] = true end
  end

  local skipped: { [string]: boolean } = { ['target'] = true, ['node_modules'] = true }
  if given.skipDirectories then
    for _, value in given.skipDirectories do skipped[value] = true end
  end

  local found, seen = {}, {}

  local function visit(path: string)
    if jess.fs.isDir(path) then
      for _, name in jess.fs.list(path) do
        if string.sub(name, 1, 1) == '.' or skipped[name] then continue end
        visit(jess.fs.join(path, name))
      end
      return
    end

    if not jess.fs.isFile(path) or not extensions[extension(path)] then return end
    -- Keep the physical filesystem path intact. On POSIX, a backslash is a legal filename
    -- character; normalizing it here can turn a real file into a nonexistent path. Logical/module
    -- path normalization happens separately in moduleKey/moduleSuffixIndex.
    if seen[path] then return end
    seen[path] = true
    table.insert(found, path)
  end

  for _, path in paths do
    -- A root that isn't there is a mistake in the paths given, not a workspace with nothing in
    -- it: finding no files would read as finding nothing wrong.
    if not jess.fs.exists(path) then error(`{path}: no such file or folder`) end

    visit(path)
  end

  table.sort(found)
  return found
end

local function moduleKey(path: string): string
  local value = normalizePath(path)
  value = string.gsub(value, '^%./', '')
  value = string.gsub(value, '%.luau?$', '')
  return (string.gsub(value, '/', '.'))
end

local function moduleSuffixIndex(files: { string }): { [string]: string }
  local index: { [string]: string } = {}
  -- files is sorted, so ambiguous suffixes resolve deterministically to the lexicographically first
  -- path. This preserves the old suffix-oriented lookup semantics without O(requires * files).
  for _, path in files do
    local normalized = normalizePath(path)
    local start = 1
    while true do
      local suffix = string.sub(normalized, start)
      if index[suffix] == nil then index[suffix] = path end
      local slash = string.find(normalized, '/', start, true)
      if not slash then break end
      start = slash + 1
    end
  end
  return index
end

local function resolveLocalModule(moduleName: string, bySuffix: { [string]: string }): string?
  local candidate = string.gsub(moduleName, '%.', '/')
  return bySuffix[`{candidate}.lua`]
    or bySuffix[`{candidate}.luau`]
    or bySuffix[`{candidate}/init.lua`]
    or bySuffix[`{candidate}/init.luau`]
end

local function workspace(paths: { string }, options: WorkspaceOptions?): Workspace
  local given: WorkspaceOptions = options or {}
  local files = collectFiles(paths, given)
  local sources, byPath, byModule = {}, {}, {}

  for _, path in files do
    local text = jess.fs.read(path)
    if not text then continue end
    if given.transform then text = given.transform(text, path) end
    local source = analyze(text, { path = path })
    table.insert(sources, source)
    byPath[path] = source
    byModule[moduleKey(path)] = source
  end

  local requireEdges, unresolvedRequires = {}, {}
  local bySuffix = moduleSuffixIndex(files)

  for _, source in sources do
    local from = source.path :: string
    for _, required in source.requires do
      if string.sub(required.module, 1, 1) == '@' or string.find(required.module, '^openmw%.') then continue end

      local target = resolveLocalModule(required.module, bySuffix)
      if target then
        table.insert(requireEdges, { from = from, to = target, module = required.module, line = required.line })
      else
        table.insert(unresolvedRequires, { from = from, module = required.module, line = required.line })
      end
    end
  end

  table.sort(requireEdges, function(left: { from: string, to: string, module: string, line: number }, right: { from: string, to: string, module: string, line: number }): boolean
    if left.from ~= right.from then return left.from < right.from end
    if left.to ~= right.to then return left.to < right.to end
    return left.line < right.line
  end)

  table.sort(unresolvedRequires, function(left: { from: string, module: string, line: number }, right: { from: string, module: string, line: number }): boolean
    if left.from ~= right.from then return left.from < right.from end
    if left.module ~= right.module then return left.module < right.module end
    return left.line < right.line
  end)

  return {
    roots = table.clone(paths),
    files = files,
    sources = sources,
    byPath = byPath,
    byModule = byModule,
    requireEdges = requireEdges,
    unresolvedRequires = unresolvedRequires,
  }
end

local function file(path: string): Source
  local text = jess.fs.read(path)
  if not text then error(`{path}: no such file`) end
  return analyze(text, { path = normalizePath(path) })
end

local function callsTo(source: Source, target: string): { CallFact }
  local found = {}
  for _, call in source.calls do
    if call.target == target then table.insert(found, call) end
  end
  return found
end

local function callsInModule(source: Source, moduleName: string): { CallFact }
  local found = {}
  for _, call in source.calls do
    if call.module == moduleName then table.insert(found, call) end
  end
  return found
end

local function resolve(source: Source, expr: Expr, depth: number?): Expr
  local left = depth or 16
  local current = expr
  local seen: { [Local]: boolean } = {}

  while left > 0 and current.kind == 'ExprLocal' do
    local target = current['local']
    if seen[target] then break end
    seen[target] = true
    local value = source.localValues[target]
    if not value then break end
    current = value
    left -= 1
  end

  if current.kind == 'ExprGroup' or current.kind == 'ExprTypeAssertion' or current.kind == 'ExprInstantiate' then
    return resolve(source, current.expr, left)
  end

  return current
end

local function resolvedTableField(source: Source, expr: Expr, name: string): Expr?
  local current = resolve(source, expr)
  if current.kind ~= 'ExprTable' then return nil end

  for _, item in current.items do
    if item.itemKind == 'record' and item.key and item.key.kind == 'ExprConstantString' and item.key.value == name then
      return item.value
    end
  end

  return nil
end

local function tableField(expr: Expr, name: string): Expr?
  if expr.kind ~= 'ExprTable' then return nil end

  for _, item in expr.items do
    if item.itemKind == 'record' and item.key and item.key.kind == 'ExprConstantString' and item.key.value == name then
      return item.value
    end
  end

  return nil
end

local function line(source: Source, number: number): string
  local first = source.lineStarts[number]
  if not first then return '' end
  local nextStart = source.lineStarts[number + 1] or (#source.source + 1)
  local text = string.sub(source.source, first, nextStart - 1)
  return (string.gsub(text, '[\r\n]+$', ''))
end

return table.freeze {
  analyze = analyze,
  file = file,
  workspace = workspace,
  files = collectFiles,
  callsTo = callsTo,
  callsInModule = callsInModule,
  qualifiedPath = qualifiedPath,
  literalString = literalString,
  tableField = tableField,
  resolvedTableField = resolvedTableField,
  resolve = resolve,
  line = line,
  normalizePath = normalizePath,
}
