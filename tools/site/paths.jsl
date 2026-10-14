--!strict
--!native
-- Where the repository's site is, from where this package sits: tools/site/ is two folders below
-- the root, and `site/` is beside `tools/`. Run from somewhere else, the scripts still find it.
local fs = require '@dream/fs'

local function repositoryRoot(): string
  local script = arg[0]
  local here = script and jess.fs.parent(script)
  local tools = here and jess.fs.parent(here)
  local root = tools and jess.fs.parent(tools)
  if root and root ~= '' then return root end

  return fs.cwd() or '.'
end

local Root = repositoryRoot()

return table.freeze {
  root = Root,
  site = jess.fs.join(Root, 'site'),
  public = jess.fs.join(Root, 'site', 'public'),
}
