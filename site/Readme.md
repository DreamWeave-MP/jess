# jess documentation site

The source of <https://DreamWeave-MP.github.io/jess>. The site is built by Jess itself using the separate `dreamweave-site` package.

From the repository root:

```sh
./buildSite                 # production build into site/public
./buildSite /tmp/jess-site  # production build into another directory
./buildSite serve           # dev build + http://127.0.0.1:1111 + watch
./buildSite serve 0.0.0.0 8080
```

`buildSite` assumes `jess` is in `$PATH`. It activates the site-generator JPK with `jess run`, renders Markdown through the Tera-shaped template layer, compiles Sass, copies static assets, and emits the search index/sitemap. There is deliberately no Zola dependency in the build path.

`./buildSite serve` is the local edit loop. It asks the same Jess generator for a dev build with a root-relative base URL, serves `site/public`, and watches content/templates/Sass/static/data/config for changes. A failed rebuild leaves the last successful site in place. The browser-facing HTTP shim uses Python 3's standard library because Jess currently exposes DreamNet transport rather than a raw TCP listener; Python does not render or transform any site content.

## Layout

- `content/_index.md` supplies searchable homepage metadata; `templates/index.html` owns the full landing-page composition.
- `content/docs/` is the documentation root (`docs_root: true`); each directory under it is a section with an `_index.md`.
- `templates/base.html` is the readable shared shell; `templates/docs/` adds the recursive docs navigation and page TOC.
- `sass/site.sass` is the shared DreamWeave shell. `sass/brand.sass` is Jess's ultraviolet brand/landing layer.
- `static/js/jess-hero.js` owns the raw WebGL2 landing hero and scroll reveals. The page remains complete without WebGL or motion.
- `data/showcase.toml` contains the homepage benchmark receipts. Keep each number tied to a named workload and its source benchmark notes.
- `dreamweave-site` is a separate package repository. `buildSite` resolves it from `JESS_SITE_PACKAGE`, then `jpk/dreamweave-site`, then an ignored local `tools/site/dreamweave-site.jpk` artifact.

## Writing docs

A docs section's `_index.md` sets `template: docs/section.html`, `page_template: docs/page.html`, and `sort_by: weight`. Page frontmatter is YAML and normally includes `title`, `description`, `weight`, and `extra.kind` (`guide`, `reference`, or `api`). Leave gaps between weights so pages can be inserted later.

Link between pages with source paths such as `[Packages](@/docs/scripting/packages.md)`. The generator resolves them to the configured base URL during the build.

API docs get a recursive project sidebar, a current-page table of contents, breadcrumbs, responsive three-column layout, search, and copy buttons for fenced code blocks. Guide pages should be task-first; reference pages should be lookup-first.

Use `{{ api_signature(value="module.function(argument)") }}` for a highlighted API signature. Use `{% usage_note(title="Compatibility") %}This behavior is optional.{% end %}` for a bordered note.

## Landing-page editorial rule

Bombast has to cash a check. The homepage uses a threat/claim only when the next beat is architecture, shipped package code, or measured data. Lyric interstitials are chapter breaks, not wallpaper.
