#!/usr/bin/env bash
# Serves playground/ and loads it in headless Chrome, as the Pages site does, then checks the
# status line says the sample text has one error. Run from the repository root, after wasm-bindgen
# wrote playground/pkg.
set -euo pipefail
port=$((20000 + RANDOM % 20000))
python3 -m http.server "$port" --bind 127.0.0.1 --directory playground >/dev/null 2>&1 &
server=$!
trap 'kill "$server"' EXIT
sleep 1
dom=$(google-chrome --headless=new --no-sandbox --disable-gpu --virtual-time-budget=10000 \
    --dump-dom "http://127.0.0.1:$port/${1:-}" 2>/dev/null)
if ! grep -q 'id="status"[^>]*>1 error<' <<<"$dom"; then
    echo "the playground did not load: status is '$(grep -o 'id="status"[^<]*' <<<"$dom")'" >&2
    exit 1
fi
