#!/usr/bin/env bash
# Serves site/ and saves screenshots of it and of three docs pages in headless Chrome, on a
# desktop and a phone width, in light and dark, into DIR, so a pull request's pages can be looked
# at without running them. Run from the repository root, after wasm-bindgen wrote playground/pkg
# and scripts/docs-site wrote site/. CHROME names another Chrome binary.
set -euo pipefail
out=${1:?usage: playground/screenshots.sh DIR}
mkdir -p "$out"
port=$((20000 + RANDOM % 20000))
# The pages' links are absolute under /jsonvet/, as on Pages, so the site is served at that path.
root=$(mktemp -d)
ln -s "$PWD/site" "$root/jsonvet"
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$root" >/dev/null 2>&1 &
server=$!
trap 'kill "$server"; rm -r "$root"' EXIT
sleep 1
for page in playground: docs-usage:docs/usage.html docs-conformance:docs/conformance.html docs-matrix:docs/matrix.html; do
    for size in desktop:1440,900 phone:390,900; do
        for scheme in light dark; do
            dark=()
            [ "$scheme" = dark ] && dark=(--force-dark-mode)
            name=${page%%:*}-${size%%:*}-$scheme
            "${CHROME:-google-chrome}" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
                --virtual-time-budget=10000 "${dark[@]}" --window-size="${size#*:}" \
                --screenshot="$out/${name#playground-}.png" "http://127.0.0.1:$port/jsonvet/${page#*:}" 2>/dev/null
        done
    done
done
ls "$out"
