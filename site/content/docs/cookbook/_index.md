---
title: 'Cookbook'
description: 'Small, practical jobs with an explanation of the contracts behind them.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 20
extra:
  kind: guide
---

These recipes start with a job and point to the reference for its full contract. They are deliberately small enough to run and change. If the terms are unfamiliar, start with [Getting Started](@/docs/getting-started/_index.md).

- [Write a JSON report](@/docs/cookbook/json-report.md): choose an explicit value shape and produce deterministic output.
- [Keep a backup when writing](@/docs/cookbook/safe-write.md): preserve a previous file before replacing it.
- [Trace a package before running it](@/docs/cookbook/inspect-package.md): inspect its payload, plan, and requested authority.
- [Send a named event](@/docs/cookbook/named-event.md): queue an application event and receive its payload.
- [Fetch a resource over HTTP](@/docs/cookbook/http-get.md): bound a request and handle its response.
- [Do work in a worker](@/docs/cookbook/worker.md): send a message and poll for a reply without blocking a frame.
- [Write and read configuration](@/docs/cookbook/configuration.md): preserve the previous file and validate decoded data.
- [Mix audio offline](@/docs/cookbook/offline-audio.md): render sample frames without an audio device.
- [Compose two packages](@/docs/cookbook/multi-package.md): run a command over a library and pack both into a workspace.
