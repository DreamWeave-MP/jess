# jess documentation site

The source of <https://DreamWeave-MP.github.io/jess>. The site is built by Jess itself: the generator is a package built on Jess, `org.dreamweave.site`, pinned in the repository's `jess.lock`, and driven by the two command targets in `tools/site`.

From the repository root:

```sh
jess workspace sync jpk/dreamweave-site                      # refresh the pinned generator from its checkout
jess workspace run --trusted jess-site-build -- target/site  # production build into target/site
jess workspace run --trusted jess-site-serve                 # dev build + http://127.0.0.1:63333/ + watch
jess workspace run --trusted jess-site-serve -- 0.0.0.0 8080
```

`tools/site` is a Package Source whose only contents are the `build` and `serve` targets. The generator renders Markdown through the Tera-shaped template layer, compiles Sass, copies static assets, and emits the search index and sitemap. There is deliberately no Zola, Node, or Python in the build path.

`serve` is the local edit loop. It asks the same generator for a dev build into `target/site-dev` with a root-relative base URL, serves it over HTTP from one nonblocking loop on `@dream/tcp`, and watches content, templates, Sass, static files, data, and the config for changes. A failed rebuild prints why and leaves the last successful site in place.

## Layout

- `content/_index.md` supplies searchable homepage metadata; `templates/index.html` owns the full landing-page composition.
- `content/docs/` is the documentation root (`docs_root: true`); each directory under it is a section with an `_index.md`.
- `templates/base.html` is the readable shared shell; `templates/docs/` adds the recursive docs navigation and page TOC.
- `sass/site.sass` is the shared DreamWeave shell. `sass/brand.sass` is Jess's ultraviolet brand and landing layer. The landing page is text and CSS: no canvas, no scripts of its own.

## Writing docs

A docs section's `_index.md` sets `template: docs/section.html`, `page_template: docs/page.html`, and `sort_by: weight`. Page frontmatter is YAML and normally includes `title`, `description`, `weight`, and `extra.kind` (`guide`, `reference`, or `api`). Leave gaps between weights so pages can be inserted later.

Link between pages with source paths such as `[Packages](@/docs/scripting/packages.md)`. The generator resolves them to the configured base URL during the build.

API docs get a recursive project sidebar, a current-page table of contents, breadcrumbs, responsive three-column layout, search, and copy buttons for fenced code blocks. Guide pages should be task-first; reference pages should be lookup-first.

Use `{{ api_signature(value="module.function(argument)") }}` for a highlighted API signature. Use `{% usage_note(title="Compatibility") %}This behavior is optional.{% end %}` for a bordered note.

Every ```` ```luau ```` block on the site is checked by `@jess/style` in the `lua_luau_style` test. Examples are run against the real binary before they are written down.

## What the site does not mention

The site documents jess: the kernel, the built-in package, and the generator that builds the site. It names no other package, no game, no engine, and no file format belonging to one. Examples use generic domains (the greeter package, a logger, a site, a build pipeline).

## Landing-page editorial rule

Bombast has to cash a check. The homepage uses a claim only when the next beat is architecture or something the binary actually prints. Lyric interstitials are chapter breaks, not wallpaper. No benchmark numbers: performance is described as mechanism in the docs.
