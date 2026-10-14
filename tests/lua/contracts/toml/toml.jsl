--!strict
-- TOML documents as tables, and the layout Luau tables forget: key order, floats, and datetimes.
local oracle, env = ...

local value: any, layout: any = jess.toml.decode [==[
zeta = 1
alpha = 2.5
flag = true
whole = 3.0
big = 9007199254740993
list = [1, 2.0, "three"]

[table.inner]
name = "x"
when = 1979-05-27T07:32:00Z

[[items]]
id = 1

[[items]]
id = 2
]==]

local document = layout[value]
local emptyValue = jess.toml.decode ''

return {
  value = value,
  keys = document.keys,
  floats = document.floats,
  listFloats = layout[value.list].floats,
  listKeys = layout[value.list].keys,
  innerKeys = layout[value.table.inner].keys,
  innerDatetimes = layout[value.table.inner].datetimes,
  rootDatetimes = document.datetimes,
  itemsFloats = layout[value.items].floats,
  -- Integers past 2^53 don't fit a Luau number: this one reads as 2^53.
  bigRounded = value.big == 2 ^ 53,
  empty = emptyValue,
  errors = {
    syntax = oracle.raises('parse', 'TOML parse error', jess.toml.decode, 'a = '),
    duplicate = oracle.raises('parse', 'duplicate key', jess.toml.decode, 'a = 1\na = 2'),
  },
}
