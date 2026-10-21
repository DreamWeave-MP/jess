---
title: 'Releasing'
description: 'How a release is built and published, and how this site deploys.'
weight: 30
extra:
  kind: guide
---

Releases are boring on purpose. You bump the version, let the gates pass on `main`, and push a tag. [StroggForge](https://github.com/DreamWeave-MP/StroggForge) does the rest: builds, signs, scans, packages, publishes the crate, and tells Discord. Nobody runs `cargo publish` by hand.

## The pipeline

`.github/workflows/release.yaml` is a few lines that call StroggForge's reusable `rustGlobalBuild.yml`:

```yaml
uses: DreamWeave-MP/StroggForge/.github/workflows/rustGlobalBuild.yml@v55
secrets: inherit
with:
  binary_names: '["jess"]'
  binary_package: jess
  include_files: "README.md,LICENSE-MIT,LICENSE-APACHE,docs"
  publish_docs: false
  cargo_publish_workspace: true
  msrv: auto
```

- `binary_package: jess` builds the binary with `cargo build --package jess --bin jess`.
- `include_files` goes into every archive next to the binary: the readme, both license texts, and a `docs` folder. The Luau types aren't shipped: `jess luau-types` writes the folder that matches the binary, and `jess init` writes it into a script folder.
- `publish_docs: false`, because this site deploys itself (below) instead of StroggForge's `cargo doc` pages. API docs live on docs.rs.
- `cargo_publish_workspace: true` publishes through StroggForge's workspace publisher, which for a one-crate workspace is one crate: `jess`.
- `msrv: auto` checks against the `rust-version` in `Cargo.toml` (1.92).

It runs on pull requests, pushes to `main`, tags, and by hand. What it does depends on which:

| Trigger | Gates | Binaries | GitHub release | crates.io | Discord |
| --- | --- | --- | --- | --- | --- |
| Pull request | yes | built, uploaded as workflow artifacts | no | dry run | no |
| Push to `main` | yes | built, signed, scanned | refreshes `development` | dry run | yes |
| Tag | yes | built, signed, scanned | creates one named after the tag | publishes | yes |

**Gates** are the checks in [Building and Testing](@/docs/contributing/building-and-testing.md#what-ci-checks): tests on every platform, fmt, pedantic clippy with `-D warnings`, `cargo audit`, and the MSRV check. Every build job waits on all of them.

**Binaries** build on macOS (ARM and Intel) and Windows, and on Linux inside an AlmaLinux 8 container, so the Linux build runs on anything with glibc 2.28 or newer. Each binary is signed with cosign, archived with the `include_files`, and scanned on VirusTotal; the scan links go into the release notes.

**The GitHub release** gets every platform's archive. StroggForge generates its notes from the git history, which is why commit subjects are written for humans: they are the changelog. There is no hand-maintained one.

## Publishing to crates.io

On a tag, once every platform built, the publisher uploads `jess` and waits for the crates.io index to show it before it declares the job done. It is rerun-safe: a version already in the index is skipped, a 429 is waited out for the window crates.io names, and a job that dies halfway picks up where it stopped when rerun.

jess takes [l3i](https://crates.io/crates/l3i) from crates.io, so a release that needs a newer l3i waits for l3i's release first. To build against an unreleased l3i locally, override it on the command line (`--config 'patch.crates-io.l3i.path="..."'`), never in a committed file.

On every other run, `cargo publish --dry-run` packages and verifies the crate, so a file missing from `include` turns up on the pull request instead of on release day. The crate ships `/src`, `/jpk/builtin`, the readme, and the two licenses; `tests/` and `tools/` stay out.

## Discord

One notification per run, sent after every other job finishes, on pushes only (not pull requests). Success says where the release is available. Any failure sends one "pipeline failed" message with a link to the run. A run cancelled because a newer push replaced it stays quiet.

## Secrets

Set these on the repository (`secrets: inherit` passes them through):

| Secret | For | Without it |
| --- | --- | --- |
| `VT_API_KEY` | VirusTotal scans | Build jobs fail on every non-PR run; the key is required. |
| `CARGO_REGISTRY_TOKEN` | crates.io | Publishing fails on tags. Needs publish rights to `jess`. |
| `DISCORD_CHANNEL_WEBHOOK` (or `DW_TOOLS_CHANNEL_WEBHOOK`) | The Discord notification | No notification. |

`GITHUB_TOKEN` is automatic.

## Cutting a release

1. **Bump the version** in `[workspace.package] version` in `Cargo.toml`. The built-in package's `jpk/builtin/jess.toml` carries its own `version`; move it when its exports changed.
2. **Run the gates locally** and let the publish dry run pass on `main`.
3. **Tag and push:** `git tag 0.2.0 && git push origin 0.2.0`. The tag name becomes the release name.
4. **Watch the run.** If publishing stops partway, rerun the job; it skips what's already up.

`Cargo.lock` is committed, so a release builds from exactly the versions the gates just tested, and the MSRV job checks those same versions on 1.92. Update it on purpose with `cargo update`, in its own `DEPS:` commit, and rerun the gates. A dependency that shows up in the lock file by accident is a dependency nobody reviewed. (`cargo install jess` users resolve fresh unless they pass `--locked`, which is worth saying in bug reports.)

## The docs site

This site isn't part of the release pipeline. `.github/workflows/docs.yaml` builds jess itself (the development build, since a docs build is not a measurement), restores the pinned site generator with `jess workspace sync --locked`, and builds the site with `jess workspace run --trusted jess-site-build`. On pull requests that touch `site/`, `tools/site/`, the workspace files, the built-in package, or `src/`, that is the check; on pushes to `main` it also deploys the result to GitHub Pages. A clean clone and a jess binary are everything the build needs: no Zola, no checkout of the generator. Docs changes go live when they merge, not when a version ships, which is why the docs describe `main`.
