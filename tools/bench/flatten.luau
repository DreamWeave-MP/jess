--!strict
--[[
Flattens an OpenMW config chain into one standalone config for the benchmark.

  jess run tools/bench/flatten.luau <output folder> <openmw.cfg> [content name to leave out]...

An empty <openmw.cfg> is OpenMW's own config, found the way OpenMW finds it.

Writes `openmw.cfg.in`: `encoding=`, every data folder and fallback archive with absolute paths, and
the content list without the names given (the tools' own outputs). The harness adds `data-local=`
per tool. It also writes `content.tsv`, each content file's name and resolved path, for the load
order's facts. Nothing in the source chain is written.
]]
local output: string?, config: string? = ...
if not output or not config then
  error('usage: jess run flatten.luau <output folder> <openmw.cfg> [content to leave out]...', 0)
end

if config == '' then jess.openmw.configPath() end

local openmw = if config == '' then jess.openmw.default() else jess.openmw.load(config)
if not openmw then error('flatten: no OpenMW config found', 0) end

local excluded: { [string]: boolean } = {}
for index = 3, select('#', ...) do
  excluded[string.lower(select(index, ...))] = true
end

-- openmw.cfg quotes a path in double quotes, escaping `&` and `"` with `&`.
local function quoted(path: string): string return `"{path:gsub('&', '&&'):gsub('"', '&"')}"` end

local lines, content = {}, {}
if openmw.encoding then table.insert(lines, `encoding={openmw.encoding}`) end

for _, folder in openmw.dataDirs do
  table.insert(lines, `data={quoted(folder)}`)
end

for _, archive in openmw.fallbackArchives do
  table.insert(lines, `fallback-archive={archive}`)
end

for _, file in openmw:content() do
  if excluded[string.lower(file.name)] then continue end

  table.insert(lines, `content={file.name}`)
  table.insert(content, `{file.name}\t{file.path or ''}`)
end

jess.fs.write(jess.fs.join(output, 'openmw.cfg.in'), table.concat(lines, '\n') .. '\n')
jess.fs.write(jess.fs.join(output, 'content.tsv'), table.concat(content, '\n') .. '\n')
