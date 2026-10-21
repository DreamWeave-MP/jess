#!/bin/bash
# First deploy of one DreamWeave package site, by hand, once: the step CI cannot take because every
# site's CI needs every other site live. Run from the repository's root after its GitHub repository
# exists with `origin` set and Pages set to deploy from the gh-pages branch.
#
#   bootstrap-site.sh <slug> <version>            e.g. bootstrap-site.sh magick 0.1.2
#
# What it does, in order: syncs the workspace offline from the sibling checkouts (the template and
# the other packages), tags <slug>-<version> at HEAD, builds the release and records it in mod.lock,
# packs the development build and the protocol files, renders the site at its real URL, checks its
# links, commits mod.lock, pushes main and the tag, creates the GitHub release with the JPK, and
# pushes public/ to gh-pages. The tag's CI run fails until every site is live; that is expected.
# Afterwards, once all six are live: `jess workspace sync --publisher <each site>`, commit jess.lock,
# flip Pages to GitHub Actions, push.
#
# JESS names the jess binary (default: the one on PATH), and TEMPLATE the Mod Template checkout
# (default: DreamWeave-Mod-Template beside the GitHub folder this repository is in). The other
# package checkouts are this repository's jpk/ siblings.
set -euo pipefail
slug=${1:?usage: bootstrap-site.sh <slug> <version>}
version=${2:?usage: bootstrap-site.sh <slug> <version>}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
JESS=${JESS:-jess}
JPK=$here/jpk
TEMPLATE=${TEMPLATE:-$here/../../DreamWeave-Mod-Template}
root=$(pwd)
tag="$slug-$version"

[ -z "$(git status --short)" ] || { echo "working tree is not clean"; exit 1; }
base=$(sed -n 's/^base_url = "\(.*\)"$/\1/p' config.toml | head -1)
[ -n "$base" ] || { echo "config.toml has no base_url"; exit 1; }
repo=$(git remote get-url origin | sed -E 's#.*github.com[:/]##; s#\.git$##')
echo "== $repo: $tag at $base"

offers=()
for folder in "$TEMPLATE" "$JPK/dreamweave-site" "$JPK/dreamweave-theme" "$JPK/magick" "$JPK/vfs" "$JPK/dream-archive"; do
  [ "$(realpath "$folder")" = "$(realpath "$root")" ] || offers+=("$folder")
done
$JESS workspace sync --offline "${offers[@]}" | grep "^lock"

git tag "$tag"
export DREAMWEAVE_BASE_URL=$base GITHUB_RUN_NUMBER=1
$JESS workspace run --trusted dreamweave-check
$JESS workspace run --trusted dreamweave-release -- "$tag"
mkdir -p "$root/.bootstrap" && cp dist/*.jpk "$root/.bootstrap/"
$JESS workspace run --trusted dreamweave-record -- dist/release.json
$JESS workspace run --trusted dreamweave-build -- "$base"
$JESS workspace run --trusted dreamweave-site -- public "$base"
$JESS workspace run --trusted dreamweave-links -- public "$base"

git add -- '*mod.lock'
git commit -q -m "RELEASE: $(sed -n 's/^title = "\(.*\)"$/\1/p' config.toml | head -1) $version"
git push origin HEAD "$tag"
gh release create "$tag" "$root"/.bootstrap/*.jpk --repo "$repo" --title "$tag" --notes "First release, published by hand to bootstrap the network; CI publishes every later one."

# public/ to gh-pages, replacing whatever is there.
rm -rf "$root/.bootstrap/pages" && git clone -q --no-checkout "$(git remote get-url origin)" "$root/.bootstrap/pages"
( cd "$root/.bootstrap/pages" && git checkout -q --orphan gh-pages && git rm -rfq . 2>/dev/null || true
  cp -r "$root/public/." . && touch .nojekyll && git add -A && git commit -q -m "Deploy $tag by hand" && git push -q -f origin gh-pages )
rm -rf "$root/.bootstrap" dist public
echo "== $repo is live at $base once Pages serves gh-pages"
