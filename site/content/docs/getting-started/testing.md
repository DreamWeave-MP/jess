---
title: 'Checking and Testing Your Work'
description: 'Choose the right check, understand source and packed tests, and work from a failure back to its cause.'
weight: 40
extra:
  kind: guide
---

Checking and testing answer different questions. A type check reads code and reports errors it can prove without running it. A test executes cases and checks their results. Neither replaces the other: types cannot prove that a file exists at runtime, and a passing example cannot exercise every possible input.

## For a script

Use `jess check` with the script path to check it and its relative requires. Use `jess run` to execute it. The [first script](@/docs/scripting/intro.md) shows both commands and a type error with its file, line, and column. Running does not automatically perform the type check.

For standalone suites, [jess test](@/docs/cli/test.md) runs test modules. The [test API](@/docs/api/test.md) describes suites, expectations, and their reports.

## For a package

After creating the greeter in [Your First Package](@/docs/getting-started/first-package.md), run:

```sh
jess package check greeter
jess package test greeter
```

The check resolves the package's requires and checks its `.jsl` files in strict mode. The test command runs the source suite, packs the package into a temporary JPK, and runs the suite against that provider too. It compares the providers' digests. A failure in both runs may therefore appear twice; look at each case's error, not just the total.

This catches a particularly annoying class of mistake: a local file exists, but the manifest didn't include it in the payload. Test the source folder; invoking package test on a JPK alone does not run its source-only tests.

## When a check fails

Start with the first reported file and location. A misspelled field may cause several later errors, so fix the earliest cause and run the check again. If an editor disagrees with the command line after a Jess upgrade, regenerate its types as described in [Editor Setup](@/docs/getting-started/editor-setup.md).

## When a test fails

Read the failing case and its error before changing code. Ask which contract failed: a return value, an error, a file or capability, or a difference between source and packed execution. Reduce the input until the failure is understandable, then add or retain a case that demonstrates the intended behavior. [Troubleshooting](@/docs/reference/troubleshooting.md) maps common errors to their causes.

For multiple packages developed together, [workspace test](@/docs/cli/workspace.md#jess-workspace-test) resolves members together. It does not remove the need for each package to declare its own dependencies.
