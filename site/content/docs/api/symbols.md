---
title: 'Generated API Symbols'
description: 'Signatures and types generated from the Jess runtime used to build this documentation.'
template: docs/symbols.html
weight: 5
extra:
  kind: api
---

This reference is generated from the declarations and source documentation in the same type surface as `jess luau-types`. The [JSON catalog](../../../api-catalog.json) contains the same entries for tools.

The signatures describe the runtime used for this documentation build. Generate types with your own Jess installation when working against a different version.

Each JSON symbol records its name, module, signature, source, and site-relative `url`. When a source document provides behavioral guidance, `reference` gives its site-relative path. The descriptions and source paths describe the generated declarations; consult the reference and implementation for behavioral details that types cannot express.

`valueExpression` is true when an export is shown as its source expression rather than an expanded inferred type. It identifies that limitation explicitly; it is not a promise that the value is undocumented or private.

Each generated reference page is also published as `index.md` beside its HTML page. Tools can read those files without rendering HTML or running JavaScript.

For a direct link to a symbol, the anchor is `symbol-` followed by the lowercase hexadecimal UTF-8 bytes of its catalog `name`.
