--!strict

local site = require '@dreamweave/site'

local root = arg[1] or 'site'
local public = arg[2] or jess.fs.join(root, 'public')
local baseUrl = arg[3]

local options = {}
if baseUrl ~= nil and baseUrl ~= '' then options.baseUrl = baseUrl end

local report = site.build(root, public, options)

print(
  string.format(
    '[jess] site complete: %d pages, %d sections, %d search indexes -> %s',
    report.pages or 0,
    report.sections or 0,
    report.search or 0,
    public
  )
)
