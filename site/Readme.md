# jess documentation site

The source of <https://DreamWeave-MP.github.io/jess>. The site is built by Jess itself: the generator is a package built on Jess, `org.dreamweave.site`, pinned in the repository's `jess.lock`, and driven by the two command targets in `tools/site`.

From the repository root:

```sh
jess workspace sync jpk/dreamweave-site                      # refresh the pinned generator from its checkout
jess workspace run --trusted jess-site-build -- target/site  # production build into target/site
jess workspace run --trusted jess-site-serve                 # dev build + http://127.0.0.1:63333/ + watch
jess workspace run --trusted jess-site-serve -- 0.0.0.0 8080
jess workspace run --trusted jess-site-navigation-check      # verify sidebar and section navigation
```

`tools/site` is a Package Source with build, serve, and navigation-check targets. The generator renders Markdown through the Tera-shaped template layer, compiles Sass, copies static assets, and emits the search index and sitemap. There is deliberately no Zola, Node, or Python in the build path.

`serve` is the local edit loop. It asks the same generator for a dev build into `target/site-dev` with a root-relative base URL, serves it over HTTP from one nonblocking loop on `@dream/tcp`, and watches site and API source files for changes. A failed rebuild prints why and leaves the last successful site in place.

Both build and serve run `jess luau-types` to derive the API catalog and need `process.spawn` in addition to their filesystem capabilities. The build writes `api-catalog.json`, creates browsable module and symbol pages, and adds signatures to search. Source comments supply generated descriptions. Existing reference pages are being migrated to source-owned documentation.

Changes to declarations appear in the generated reference on the next build; no signature baseline needs updating.

Migrated reference pages keep their frontmatter and public paths in `content/docs/api/`; their prose is in an `@api-document` block in the corresponding JSL source. The first line names the page slug and its generated module, for example `@api-document jpk @jess/jpk`. The build combines that prose with signatures from the current declarations. A missing source document fails the build. Keep behavior and examples in source comments and avoid copying signatures into those comments.

To place a generated declaration beside its explanation, use `{{ api_symbol(value="jess_runtime_Worker.send") }}` in the source document. The value is the catalog's symbol name, not a copied signature. References to removed symbols fail the build so an obsolete explanation cannot silently remain in the reference.

The tutorial test suite uses `JESS_DOCS_RUNTIME` as the runtime for its standalone-executable case. From the root, set it to the same executable as `jess` on `PATH` and run `jess test` on `tools/site/tests/tutorials.jsl` with `process.spawn`, `process.environment`, `filesystem.read`, and `filesystem.write` granted. The docs workflow does this and tests the HTTP recipe against loopback.

## Layout

- `content/_index.md` supplies searchable homepage metadata; `templates/index.html` owns the full landing-page composition.
- `content/docs/` is the documentation root (`docs_root: true`); each directory under it is a section with an `_index.md`.
- `templates/base.html` is the readable shared shell; `templates/docs/` adds the recursive docs navigation and page TOC.
- `sass/site.sass` is the shared DreamWeave shell. `sass/brand.sass` is Jess's ultraviolet brand and landing layer. The landing page is text and CSS: no canvas, no scripts of its own.

## Writing docs

A docs section's `_index.md` sets `template: docs/section.html`, `page_template: docs/page.html`, and `sort_by: weight`. Page frontmatter is YAML and normally includes `title`, `description`, `weight`, and `extra.kind` (`guide`, `reference`, or `api`). Leave gaps between weights so pages can be inserted later.

Link between pages with source paths such as `[Packages](@/docs/scripting/packages.md)`. The generator resolves them to the configured base URL during the build.

API docs get a recursive project sidebar, a current-page table of contents, breadcrumbs, responsive three-column layout, search, and copy buttons for fenced code blocks. Guide pages should be task-first; reference pages should be lookup-first.

CLI references use `{{ api_signature(value="command arguments") }}` for highlighted command syntax. API signatures come from declarations. Use `{% usage_note(title="Compatibility") %}This behavior is optional.{% end %}` for a bordered note.

Every ```` ```luau ```` block in site Markdown and source API documents is checked by `@jess/style` in the `lua::luau_style` tests. Examples are run against the real binary before they are written down.

## What the site does not mention

The site documents jess: the kernel, the built-in package, and the generator that builds the site. It names no other package, no game, no engine, and no file format belonging to one. Examples use generic domains (the greeter package, a logger, a site, a build pipeline).

## Landing-page editorial rule

Bombast has to cash a check. The homepage uses a claim only when the next beat is architecture or something the binary actually prints. Lyric interstitials are chapter breaks, not wallpaper. No benchmark numbers: performance is described as mechanism in the docs.
