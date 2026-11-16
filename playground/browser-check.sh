#!/usr/bin/env bash
# Serves site/ and loads it in headless Chrome, as the Pages site does, then checks the verdict line
# says the sample text parses but a rule flags it, and that a docs page loads and runs its scripts under
# its CSP. Run from the repository root, after wasm-bindgen wrote playground/pkg and
# scripts/docs-site wrote site/. CHROME names another Chrome binary.
set -euo pipefail
port=$((20000 + RANDOM % 20000))
# The pages' links are absolute under /jsonvet/, as on Pages, so the site is served at that path.
root=$(mktemp -d)
ln -s "$PWD/site" "$root/jsonvet"
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$root" >/dev/null 2>&1 &
server=$!
trap 'kill "$server"; rm -r "$root"' EXIT
sleep 1
dump() {
    "${CHROME:-google-chrome}" --headless=new --no-sandbox --disable-gpu --virtual-time-budget=10000 \
        --dump-dom "http://127.0.0.1:$port/jsonvet/$1" 2>/dev/null
}
dom=$(dump "${1:-}")
if ! grep -q 'id="status"[^>]*><b>Parses as JSON5, but a rule found 1 error\.</b>' <<<"$dom"; then
    echo "the playground did not load: status is '$(grep -o 'id="status".\{0,80\}' <<<"$dom")'" >&2
    exit 1
fi
# Starlight's inline script sets the theme on <html>, and its search script enables the search
# button, so this also shows the page's CSP let them run.
dom=$(dump docs/usage.html)
if ! grep -q '<h1 id="_top"' <<<"$dom" || ! grep -q '<html[^>]* data-theme="' <<<"$dom" \
    || grep -q '<button data-open-modal disabled' <<<"$dom"; then
    echo "docs/usage.html did not load, or its theme or search script did not run" >&2
    exit 1
fi
