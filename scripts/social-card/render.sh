#!/usr/bin/env bash
# Regenerates playground/og.png: a real `jsonvet` run on staging.json5, recorded by VHS in its
# container, then squeezed by oxipng. Run from the repository root after `cargo build --release`
# (CARGO_TARGET_DIR names another target directory). Needs docker.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
bin=$(cd "${CARGO_TARGET_DIR:-$root/target}/release" && pwd)
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
user=$(id -u):$(id -g)
vhs=ghcr.io/charmbracelet/vhs:v0.11.0@sha256:9d5fc3dc0c160b0fb1d2212baff07e6bdf3fa9438c504a3237484567302fcf93
docker run --rm --user "$user" -e HOME=/tmp -w /out -v "$here:/work:ro" -v "$bin:/opt/bin:ro" \
    -v "$out:/out" -v "$here/card.tape:/tmp/card.tape:ro" "$vhs" /tmp/card.tape
# Pinned by tag: oxipng's image has no digest list here worth tracking.
docker run --rm --user "$user" -v "$out:/out" ghcr.io/shssoichiro/oxipng:v9.1.5 \
    -o max --strip safe --out /out/og.png /out/card.png
cp "$out/og.png" "$root/playground/og.png"
file "$root/playground/og.png"
