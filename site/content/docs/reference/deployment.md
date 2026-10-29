---
title: 'Deployment and Operations'
description: 'Choose an artifact, check platform requirements, and operate an application without confusing its data with its runtime.'
weight: 50
extra:
  kind: guide
---

Choose the artifact and the machine together. A successful build on one machine does not establish that an executable, a graphics driver, or a sound device works on another.

## Artifacts and platforms

| Artifact | Destination needs | What it carries |
| --- | --- | --- |
| Package Source | Jess, the source and its dependencies | Editable package files and a manifest. |
| JPK | Jess and whatever dependencies its host resolves | One immutable package. |
| `.jsw` | A compatible Jess binary | A closed workspace of runnable targets and packages; it runs offline. |
| Standalone executable | A compatible OS, architecture, and system libraries | A Jess runtime carrying the packed workspace; no separate Jess installation. |

The [installation page](@/docs/getting-started/install.md#from-a-release-archive) lists the published release archives and their Linux glibc minimums. [Workspace packing](@/docs/cli/workspace.md#jess-workspace-pack) describes standalone construction, including macOS signing. The included runtime determines the executable's platform; packing on one system does not make an arbitrary runtime portable to another.

Release archives are the verified distribution builds. A source build uses the toolchain and options of the person building it; Jess marks builds made without the verified toolchain, and the benchmark harness refuses them. See [Installing from a git checkout](@/docs/getting-started/install.md#from-a-git-checkout) and [Benchmarks](@/docs/contributing/benchmarks.md). Android and other platforms without a published artifact should not be assumed to have a supported release merely because a source build might be possible.

## Data, configuration, and credentials

An application's files and state are its own design. A packed workspace freezes code and payload; it is not an application data directory, database, or backup. Choose writable paths deliberately and document how to migrate, back up, and restore them. [Write and Read Configuration](@/docs/cookbook/configuration.md) shows validation and a local backup, not a complete persistence system.

Jess's own installer keeps commands and install records under its install root; [jess install](@/docs/cli/install.md#the-install-root) describes it. Do not assume that root is where an application stores its data. Environment access requires `process.environment`; keep credentials out of command lines, logs, and packed payloads, and use the host's appropriate secret mechanism. The package still needs the capability to read what it uses.

## Running and stopping

For a long-running target, a supervisor such as systemd can start the executable with an explicit working directory, user, environment, and restart policy. Those choices belong to the deployment: a relative application path resolves from the working directory, and a process receives the operating-system permissions of its account. A Jess capability is an authority gate inside Jess, not an OS sandbox or a substitute for filesystem permissions and process isolation.

For example, a system service for a standalone headless application might use the following unit after an administrator has installed the executable, created the service account and data directory, and chosen suitable permissions:

```ini
[Unit]
Description=Example Jess application
After=network.target

[Service]
Type=simple
User=example
WorkingDirectory=/var/lib/example
ExecStart=/opt/example/bin/example-app
Restart=on-failure
RestartSec=5s
TimeoutStopSec=30s

[Install]
WantedBy=multi-user.target
```

This is a unit file, not a complete installation procedure. Use the application's actual paths and account, ensure it has the capabilities and OS permissions it needs, and choose limits and restart behavior for its workload. `network.target` orders startup; it does not guarantee that a remote service is reachable. Keep credentials out of the unit file and command line. Test shutdown and recovery before enabling automatic restart.

On Unix, the first `SIGINT` or `SIGTERM` requests that an application stop and run its shutdown handlers; an inherited ignored signal remains ignored, and a second signal terminates immediately. On Windows, the first Ctrl+C or Ctrl+Break requests shutdown and a second is passed on. When the application loop observes an interrupt, it returns status 130 after teardown. Other platforms use their default process behavior. These are application-target behaviors; a command target runs to completion. [Applications](@/docs/scripting/applications.md#running-and-stopping) and [jess application](@/docs/api/application.md) cover the lifecycle and exit status.

Jess writes application diagnostics to stderr. Structured logs from [`@jess/log`](@/docs/api/log.md) have a host-owned sink; the command-line host writes JSON records to stderr. Capture both output streams, preserve exit status, and configure the supervisor's retention and restart limits for the workload. A restart is not data recovery: an application must define what state is durable and how interrupted work resumes.

## Headless and device-dependent work

An application that does not request a window can run headless. Window creation requires a display; GPU work needs a usable adapter or a software implementation; audio output can use a null device when no sound system is available. [Installation: runtime requirements](@/docs/getting-started/install.md#what-it-needs-at-run-time) and [Troubleshooting](@/docs/reference/troubleshooting.md#gpus-windows-and-sound) describe the current behavior. Test on the actual destination, including its drivers and permissions.
