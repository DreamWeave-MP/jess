---
title: 'Global Options'
description: 'The options every command takes: --format, --help, --version; and what every command prints, and how it fails.'
weight: 5
extra:
  kind: reference
---

{{ api_signature(value="jess [--format <FORMAT>] [COMMAND]") }}

With no command, `jess` prints its help: the commands, and two to start with. A loose script has no automatic application configuration. Workspace commands discover `jess-workspace.toml` and use its lock and package cache; install and uninstall keep records under the install root.

These work on every command: `jess --format json package inspect greeter` and `jess package inspect greeter --format json` are the same. Put Jess options before arguments intended for a script or target. Once `jess run` or `jess package run` starts collecting trailing arguments, later options belong to that script or target; `--` also separates arguments passed through to it.

| Option | Value | Default | Does |
| --- | --- | --- | --- |
| `--format <FORMAT>` | `yaml`, `json`, `toml` | `yaml` | The output document's format. JSON is compact, one line. |
| `-h`, `--help` | | | Help for the command it follows. `jess <command> --help` is the long form, with every value explained; `-h` is the summary. |
| `-V`, `--version` | | | Print `jess <version>`. A subcommand prints its own name, such as `jess-package 0.3.0`. A build without cross-language LTO says so after the version, and the benchmark harness refuses it. |

## Output

A command that reports something prints one document, in the `--format` you asked for, to stdout. Scripts are the exception: `jess run` and a command target print whatever the script prints, as it prints it, and nothing else.

YAML is the default because it reads as a report. JSON is one line, for a program that parses it. TOML is for pasting into a file that is already TOML:

```text
$ jess --format json luau-types types
{"folder":"types","definitions":"types/jess.d.jsl","files":24,"removed":[],"aliases":{"dream":"types/dream","jess":"types/jess"}}
```

## Errors and exit codes

Errors print to stderr as `error: <message>`; diagnostics an application reports go there too, one line each, as they happen. Exit code `0` is success, `1` an error or a failure the command reports on purpose (a failed test, a type error, a capability it declares and you didn't grant), `2` a command line jess couldn't parse. A script's first returned number, an integer from 0 to 255, is its exit status, and an application's is the one it passed to `application.exit`.

## Environment

Two variables for [`jess workspace sync`](@/docs/cli/workspace.md#jess-workspace-sync) and [`jess install`](@/docs/cli/install.md): `JESS_OFFLINE=1` asks no publisher, as `--offline` does, and `JESS_REGISTRY_URL` names the package index, as `--index` does. `JESS_INSTALL_ROOT` names the install root for `jess install` and `jess uninstall`, as `--root` does; without it, they use `.jess` in your home folder, which is where they read `HOME` (`USERPROFILE` on Windows). `jess install` also reads `PATH` to warn when what it installs isn't on it. A script reads environment variables through `@dream/process`, and only when granted `process.environment`.
