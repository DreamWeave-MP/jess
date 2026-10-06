---
title: 'Releasing'
description: 'How a release is built and published.'
weight: 30
extra:
  kind: guide
---

Releases are boring on purpose. Exciting releases are how you end up publishing all but one crate at 3 AM. You bump a version, write the changelog, and push a tag. [StroggForge](https://github.com/DreamWeave-MP/StroggForge) does the rest: builds, signs, scans, packages, publishes every crate in order, and tells Discord.

## The pipeline

`.github/workflows/release.yaml` is a few lines that call StroggForge's reusable `rustGlobalBuild.yml@v54`:

```yaml
uses: DreamWeave-MP/StroggForge/.github/workflows/rustGlobalBuild.yml@v54
secrets: inherit
with:
  binary_names: '["jess"]'
  binary_package: jess-cli
  include_files: "README.md,LICENSE-MIT,LICENSE-APACHE,docs"
  publish_docs: false
  cargo_publish_workspace: true
  msrv: auto
```

- `binary_package: jess-cli` builds the binary with `cargo build --package jess-cli --bin jess` from the repository root.
- `include_files` goes into every archive next to the binary: the readme, both license texts, and the `docs/` folder, which is down to `save-strip.md`. The Luau types aren't shipped: `jess luau-types` writes the folder that matches the binary, and `jess init` writes it into a script folder.
- `publish_docs: false`, because this site deploys itself (below) instead of StroggForge's `cargo doc` pages. API docs live on docs.rs.
- `cargo_publish_workspace: true` publishes every crate in the workspace, not one crate per binary.
- `msrv: auto` checks against the `rust-version` in `Cargo.toml` (1.92).

It runs on pull requests, pushes to `main`, tags, and by hand. What it does depends on which:

| Trigger | Gates | Binaries | GitHub release | crates.io | Discord |
| --- | --- | --- | --- | --- | --- |
| Pull request | yes | built, uploaded as workflow artifacts | no | dry run | no |
| Push to `main` | yes | built, signed, scanned | refreshes `development` | dry run | yes |
| Tag | yes | built, signed, scanned | creates one named after the tag | publishes | yes |

**Gates** are the checks in [Building and Testing](@/docs/contributing/building-and-testing.md#what-ci-checks): tests on four platforms, fmt, pedantic clippy with `-D warnings`, `cargo audit`, and the MSRV check. Every build job waits on all of them.

**Binaries** build on macOS (ARM and Intel) and Windows, and on Linux inside an AlmaLinux 8 container, so the Linux build runs on anything with glibc 2.28 or newer. Each binary is signed with cosign, archived with the `include_files`, and scanned on VirusTotal; the scan links go into the release notes.

**The GitHub release** gets every platform's archive. StroggForge generates its notes from the git history, which is why commit subjects are written for humans: they are the changelog. There is no hand-maintained one.

## Publishing to crates.io

On a tag, once every platform built, `cargo-publish-workspace` runs StroggForge's `publish_workspace.py`. It publishes one crate at a time, dependencies first, which for jess means:

`jess-macros`, `jess-io`, `jess-luau-types`, `jess-nif`, `jess-plugin`, `jess-omwaddon`, `jess-openmw`, `jess-assets`, `jess-save`, `jess`, `jess-lua`, `jess-cli`.

Why not just `cargo publish --workspace`? Because crates.io only lets an account publish a handful of *new* crates before it starts answering 429, and a half-published workspace can't be retried with it. The script handles both:

- **Rerun-safe.** A crate whose version is already in the crates.io index is skipped. If the job dies halfway, rerun it and it picks up where it stopped.
- **Rate limits are waited out.** On a 429 it sleeps for the window crates.io names (ten minutes if it doesn't say), then retries that crate, up to 30 times. The first release of every crate takes a while. Let it.
- **It waits for the index.** After each upload it polls until the crate shows up (up to 15 minutes) before building the next crate against it. `cargo publish` alone gives up after a minute.

The job has a four-hour timeout and never runs twice at once for the same repository.

On every other run, `cargo publish --workspace --dry-run` packages and verifies all crates against each other, so packaging mistakes turn up on the pull request instead of on release day.

## Discord

One notification per run, sent after every other job finishes, on pushes only (not pull requests). Success says where the release is available ("GitHub and crates.io" for a tag that published). Any failure sends one "pipeline failed" message with a link to the run. A run cancelled because a newer push replaced it stays quiet.

## Secrets

Set these on the repository (`secrets: inherit` passes them through):

| Secret | For | Without it |
| --- | --- | --- |
| `VT_API_KEY` | VirusTotal scans | Build jobs fail on every non-PR run; the key is required. |
| `CARGO_REGISTRY_TOKEN` | crates.io | Publishing fails on tags. Needs publish rights to every crate in the workspace. |
| `DISCORD_CHANNEL_WEBHOOK` (or `DW_TOOLS_CHANNEL_WEBHOOK`) | The Discord notification | No notification. |

`GITHUB_TOKEN` is automatic.

## Cutting a release

1. **Bump the version.** Every crate shares `[workspace.package] version` in the root `Cargo.toml`. The path dependencies between crates also carry a `version = "..."` (crates.io needs it), in the root manifest and every `libs/*/Cargo.toml`. Change them all together; `grep -rn 'version = "0.1.0"' --include=Cargo.toml .` finds them. They must match, or the publish dry run fails.
2. **Run the gates locally** and let the publish dry run pass on `main`.
3. **Tag and push:** `git tag 0.2.0 && git push origin 0.2.0`. The tag name becomes the release name.
4. **Watch the run.** If publishing stops partway, rerun the job; it skips what's already up.

`Cargo.lock` is committed, so a release builds from exactly the versions the gates just tested, and the MSRV job checks those same versions on 1.92. Update it on purpose with `cargo update`, in its own `DEPS:` commit, and rerun the gates. A dependency that shows up in the lock file by accident is a dependency nobody reviewed. (`cargo install jess-cli` users resolve fresh unless they pass `--locked`, which is worth saying in bug reports.)

## The docs site

This site isn't part of the release pipeline. `.github/workflows/docs.yaml` runs `zola check --skip-external-links` and `zola build` (the `ghcr.io/getzola/zola:v0.22.1` image) on pull requests that touch `site/`, and on pushes to `main` it also deploys `site/public` to GitHub Pages. Docs changes go live when they merge, not when a version ships. That's why the docs describe `main`.
