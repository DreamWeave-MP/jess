--!strict
--!native
-- The rules, by id: what each enforces and how loud it is. There is no suppression comment: the
-- style says to fix the code, not to quiet the checker.

export type Severity = 'error' | 'warning'

export type Rule = {
  read severity: Severity,
  read summary: string,
}

export type Finding = {
  rule: string,
  severity: Severity,
  message: string,
  line: number,
  column: number,
  endLine: number,
  endColumn: number,
}

export type Options = {
  --- 'jess' (the default) for jess's Luau; 'lua' for s3stash's Lua, where hoisting globals into
  --- PascalCase locals is the style rather than a mistake.
  profile: ('jess' | 'lua')?,
  --- Hot comments the file has to start with, such as { 'strict', 'native' }.
  directives: { string }?,
  --- Report every `any` type.
  noAny: boolean?,
  --- Parse definition-file syntax (`declare`, `declare extern type`), or not; without either, a
  --- source that only parses with it is taken for a definitions file.
  declarations: boolean?,
  --- Only these rules, by id; every rule without it.
  only: { string }?,
  --- The source is an excerpt, such as a block on a docs page: what its module does with a local
  --- isn't known, so no local is taken for a module constant.
  fragment: boolean?,
}

local Rules: { [string]: Rule } = {
  ['syntax'] = {
    severity = 'error',
    summary = 'The source parses as Luau.',
  },
  ['pcall.comment'] = {
    severity = 'error',
    summary = 'Every pcall and xpcall has a comment right above it naming what throws.',
  },
  ['layout.indentation'] = {
    severity = 'error',
    summary = 'Lines are indented with spaces, two per level.',
  },
  ['layout.compound'] = {
    severity = 'error',
    summary = 'Adjacent compound statements are set apart by a blank line when either spans several lines.',
  },
  ['quotes.double'] = {
    severity = 'error',
    summary = 'Strings use single quotes unless they hold one.',
  },
  ['quotes.backtick'] = {
    severity = 'error',
    summary = 'Backticks are for interpolation only.',
  },
  ['hoisting'] = {
    severity = 'error',
    summary = 'No local only renames a Luau global or library function.',
  },
  ['naming.case'] = {
    severity = 'error',
    summary = 'Module constants are PascalCase; every other local is camelCase.',
  },
  ['naming.constant'] = {
    severity = 'warning',
    summary = 'A module-level local holding a literal that nothing changes is a constant, and PascalCase.',
  },
  ['naming.words'] = {
    severity = 'warning',
    summary = 'Names are full words: no single letters anywhere, no abbreviations.',
  },
  ['truthiness'] = {
    severity = 'warning',
    summary = 'A value that cannot be false is tested for truth, not compared with nil.',
  },
  ['locals.group'] = {
    severity = 'warning',
    summary = 'Adjacent empty tables are declared on one line, except one filled by assignment, which is declared alone so Luau can size it.',
  },
  ['locals.mixed'] = {
    severity = 'warning',
    summary = 'Uninitialized locals are declared apart from initialized ones.',
  },
  ['guard.else'] = {
    severity = 'warning',
    summary = 'No else after a branch that returns, breaks, continues, or errors.',
  },
  ['guard.nested'] = {
    severity = 'warning',
    summary = 'A body that is one condition around the work becomes a guard clause and the work.',
  },
  ['directives.strict'] = {
    severity = 'error',
    summary = 'The file starts with --!strict, where the caller requires it.',
  },
  ['directives.native'] = {
    severity = 'error',
    summary = 'The file starts with --!native, where the caller requires it.',
  },
  ['types.any'] = {
    severity = 'error',
    summary = 'No `any` type, where the caller requires it.',
  },
}

for _, rule in Rules do
  table.freeze(rule)
end

return table.freeze { Rules = table.freeze(Rules) }
