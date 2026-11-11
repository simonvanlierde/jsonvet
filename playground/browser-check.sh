#!/usr/bin/env bash
# Serves playground/ and loads it in headless Chrome, as the Pages site does, then checks the
# verdict line says the sample text needs one correction, and that a docs page loads with its
# theme script. Run from the repository root, after wasm-bindgen wrote playground/pkg and
# scripts/docs-site/build.mjs wrote playground/docs. CHROME names another Chrome binary.
set -euo pipefail
port=$((20000 + RANDOM % 20000))
python3 -m http.server "$port" --bind 127.0.0.1 --directory playground >/dev/null 2>&1 &
server=$!
trap 'kill "$server"' EXIT
sleep 1
dump() {
    "${CHROME:-google-chrome}" --headless=new --no-sandbox --disable-gpu --virtual-time-budget=10000 \
        --dump-dom "http://127.0.0.1:$port/$1" 2>/dev/null
}
dom=$(dump "${1:-}")
if ! grep -q 'id="status"[^>]*><b>1 correction\.</b>' <<<"$dom"; then
    echo "the playground did not load: status is '$(grep -o 'id="status".\{0,80\}' <<<"$dom")'" >&2
    exit 1
fi
# theme.js names the theme button once the page has loaded, so this also shows the CSP let it run.
dom=$(dump docs/usage.html)
if ! grep -q '<h1 id="jsonvet-reference">' <<<"$dom" || ! grep -q 'aria-label="Switch to the dark theme"' <<<"$dom"; then
    echo "docs/usage.html did not load, or its theme script did not run" >&2
    exit 1
fi
