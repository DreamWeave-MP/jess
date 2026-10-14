--!strict
-- Plain file access: reads, writes with optional backups, folders, and path helpers.
local oracle, env = ...

local nested = jess.fs.join(env.temp, 'a', 'b')
jess.fs.createDir(nested)
local file = jess.fs.join(nested, 'Notes.TXT')
local missing = {
  read = jess.fs.read(file),
  size = jess.fs.size(file),
  modified = jess.fs.modified(file),
  exists = jess.fs.exists(file),
  removed = jess.fs.remove(file),
}

local first = jess.fs.write(file, 'hello')
local binary = jess.fs.write(file, buffer.fromstring 'bytes\0!')
local backedUp = jess.fs.write(file, 'again', { backup = true })
local deeper = jess.fs.write(jess.fs.join(env.temp, 'new', 'folder', 'made.txt'), 'x')

local copy = jess.fs.join(nested, 'copy.txt')
jess.fs.copy(file, copy)
jess.fs.replace(copy, 'replaced')

return {
  missing = missing,
  first = first,
  binary = binary,
  backedUp = backedUp,
  deeper = deeper,
  contents = {
    file = jess.fs.read(file),
    backup = jess.fs.read(file .. '.bak'),
    copy = jess.fs.read(copy),
  },
  -- Replaced through a temporary file and a rename: nothing else is left in the folder.
  replaced = { contents = jess.fs.read(copy), entries = #jess.fs.list(nested) },
  size = jess.fs.size(file),
  modified = type(jess.fs.modified(file)),
  kinds = {
    dirIsDir = jess.fs.isDir(nested),
    dirIsFile = jess.fs.isFile(nested),
    fileIsFile = jess.fs.isFile(file),
    fileExists = jess.fs.exists(file),
  },
  list = oracle.sorted(jess.fs.list(nested)),
  removed = { jess.fs.remove(copy), jess.fs.remove(copy) },
  paths = {
    fileName = jess.fs.fileName(file),
    parent = jess.fs.parent(file),
    rootParent = jess.fs.parent '/',
    noName = jess.fs.fileName '/',
    -- Joined with the platform's separator.
    join = string.gsub(jess.fs.join('a', 'b', 'c.txt'), '\\', '/'),
  },
  cwd = type(jess.fs.cwd()),
  errors = {
    copyMissing = oracle.raises(
      'missing',
      'failed to copy',
      jess.fs.copy,
      jess.fs.join(env.temp, 'none'),
      copy
    ),
    listMissing = oracle.raises(
      'missing',
      'failed to list',
      jess.fs.list,
      jess.fs.join(env.temp, 'none')
    ),
    readFolder = oracle.raises('argument', 'failed to read', jess.fs.read, nested),
    replaceMissingFolder = oracle.raises(
      'missing',
      'failed to write',
      jess.fs.replace,
      jess.fs.join(env.temp, 'none', 'x.txt'),
      'x'
    ),
    writeType = oracle.raises(
      'argument',
      'expected a buffer or string, not table',
      jess.fs.write,
      file,
      {}
    ),
  },
}
