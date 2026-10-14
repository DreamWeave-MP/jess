# jess documentation site

The source of <https://DreamWeave-MP.github.io/jess>. The site is built by Jess itself using the separate `dreamweave-site` package, driven by the two command targets in `tools/site`.

From the repository root:

```sh
jess package run --trusted --package-path jpk tools/site                      # production build into site/public
jess package run --trusted --package-path jpk tools/site -- /tmp/jess-site    # production build into another directory
jess package run --trusted --target serve --package-path jpk tools/site        # dev build + http://127.0.0.1:63333/ + watch
jess package run --trusted --target serve --package-path jpk tools/site -- 0.0.0.0 8080
```

`tools/site` is a Package Source whose only contents are the `build` and `serve` targets; it depends on `org.dreamweave.site`, which `--package-path jpk` finds in the `jpk/dreamweave-site` checkout. The generator renders Markdown through the Tera-shaped template layer, compiles Sass, copies static assets, and emits the search index and sitemap. There is deliberately no Zola, Node, or Python in the build path.

`serve` is the local edit loop. It asks the same generator for a dev build into `target/site-dev` with a root-relative base URL, serves it over HTTP from one nonblocking loop on `@dream/tcp`, and watches content, templates, Sass, static files, data, and the config for changes. A failed rebuild prints why and leaves the last successful site in place.

## Layout

- `content/_index.md` supplies searchable homepage metadata; `templates/index.html` owns the full landing-page composition.
- `content/docs/` is the documentation root (`docs_root: true`); each directory under it is a section with an `_index.md`.
- `templates/base.html` is the readable shared shell; `templates/docs/` adds the recursive docs navigation and page TOC.
- `sass/site.sass` is the shared DreamWeave shell. `sass/brand.sass` is Jess's ultraviolet brand/landing layer, including the hero's still.
- `static/js/jess-hero.js` owns the landing hero, a three.js scene (`static/js/vendor/`, MIT) with the dashboard reactor, and the scroll reveals. The page remains complete without WebGL or motion.
- `data/showcase.toml` contains the homepage numbers. Keep each number tied to a named workload and the bench it came from.
- `dreamweave-site` is a separate package repository, checked out at `jpk/dreamweave-site`.
## Writing docs

A docs section's `_index.md` sets `template: docs/section.html`, `page_template: docs/page.html`, and `sort_by: weight`. Page frontmatter is YAML and normally includes `title`, `description`, `weight`, and `extra.kind` (`guide`, `reference`, or `api`). Leave gaps between weights so pages can be inserted later.

Link between pages with source paths such as `[Packages](@/docs/scripting/packages.md)`. The generator resolves them to the configured base URL during the build.

API docs get a recursive project sidebar, a current-page table of contents, breadcrumbs, responsive three-column layout, search, and copy buttons for fenced code blocks. Guide pages should be task-first; reference pages should be lookup-first.

Use `{{ api_signature(value="module.function(argument)") }}` for a highlighted API signature. Use `{% usage_note(title="Compatibility") %}This behavior is optional.{% end %}` for a bordered note.

## Landing-page editorial rule

Bombast has to cash a check. The homepage uses a threat/claim only when the next beat is architecture, shipped package code, or measured data. Lyric interstitials are chapter breaks, not wallpaper.
