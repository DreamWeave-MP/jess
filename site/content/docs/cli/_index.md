---
title: 'CLI Reference'
description: 'Every jess command and flag, one page per command group.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 30

extra:
  kind: reference
---

Lookup-first: every subcommand, every flag, what it prints. [Getting Started](@/docs/getting-started/_index.md) shows these commands in context; this is the part you search.

One binary, nine commands: `run`, `test`, `check`, `pack`, `package`, `workspace`, `init`, `luau-types`, and the two that generate things for your shell, `completions` and `man`. Every command that prints a document takes [`--format`](@/docs/cli/global-options.md). A `.jsl` path where a command would go, `jess script.jsl`, [runs the script as a program](@/docs/cli/run.md#running-a-script-as-a-program), which is what a shebang line does.
