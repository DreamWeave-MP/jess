--!strict
-- Rust regexes over bytes, with `string.find`'s positions: 1-based, inclusive ends.
local oracle, env = ...

local words = jess.regex.new '(?P<word>[a-z]+)(\\d)?'
local Text = 'abc1 def xyz9'

local matches = {}
for found, start, finish in words:matches(Text) do
  table.insert(matches, { found, start, finish })
end

local empties = {}
for found, start, finish in jess.regex.new('x*'):matches 'ab' do
  table.insert(empties, { found, start, finish })
end

local caseless =
  jess.regex.new('^B.C$', { caseInsensitive = true, multiLine = true, dotAll = true })

local function upper(captures: { [number | string]: string }): string | false
  if captures.word == 'def' then return false end
  return string.upper(captures[0])
end

local function counted(...: any): { any } return { ... } end

return {
  pattern = words.pattern,
  tostring = tostring(words),
  isMatch = { words:isMatch(Text), words:isMatch '123' },
  find = {
    first = counted(words:find(Text)),
    fromInit = counted(words:find(Text, 5)),
    negativeInit = counted(words:find(Text, -4)),
    pastEnd = counted(words:find(Text, 100)),
    none = counted(words:find '123'),
  },
  match = { words:match(Text), words:match(Text, 6) },
  captures = {
    first = words:captures(Text),
    unmatchedGroup = words:captures(Text, 6),
    none = words:captures '!!!' == nil,
  },
  matches = matches,
  emptyMatches = empties,
  replace = {
    first = counted(words:replace(Text, '<$word>')),
    all = counted(words:replaceAll(Text, '[${word}$2]$$')),
    limited = counted(words:replaceAll(Text, '_', 2)),
    byFunction = counted(words:replaceAll(Text, upper)),
    none = counted(words:replace('!!!', 'x')),
  },

  split = {
    all = jess.regex.new('[,;]\\s*'):split 'a, b;c,,d',
    limited = jess.regex.new(','):split('a,b,c,d', 2),
  },
  options = { caseless:isMatch 'x\nb\nc\ny', jess.regex.new('^B.C$'):isMatch 'x\nb\nc\ny' },
  bytes = counted(jess.regex.new('(?-u:\\xff)'):find 'a\255b'),
  escape = jess.regex.escape 'a.b*c(d)',
  errors = {
    pattern = oracle.raises('parse', 'regex parse error', jess.regex.new, '('),
    replacement = oracle.raises(
      'argument',
      'a replacement is a string or a function, not boolean',
      words.replace,
      words,
      Text,
      true
    ),
    functionResult = oracle.raises(
      'argument',
      'a replacement function returns a string, nil, or false, not number',
      words.replace,
      words,
      Text,
      function() return 1 end
    ),
  },
}
