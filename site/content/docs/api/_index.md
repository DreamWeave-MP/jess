---
title: 'Luau API Reference'
description: 'Lookup-first reference for every module, type, and function in the jess Luau API.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 60

extra:
  kind: api
---

One page per module or type. Each entry leads with the signature and a one-line summary, then behavior, parameters, returns, errors, and an example that runs. This section is for looking things up, not for reading front to back; [Scripting](@/docs/scripting/_index.md) is the part that explains how the pieces fit.

Option tables, the last argument most functions take, reject keys they don't take: the error names the key and lists the ones the function does take (`unknown Plugin:save option 'bakup': expected backup`), so a typo fails instead of quietly doing the default.

The examples are type-checked against the types `jess luau-types` writes, under `--!strict`, and run against test files. If one disagrees with the jess you have, the jess you have wins, and the page is wrong.
