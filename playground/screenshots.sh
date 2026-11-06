#!/usr/bin/env bash
# Serves playground/ and saves screenshots of it in headless Chrome, on a desktop and a phone
# width, in light and dark, into DIR, so a pull request's page can be looked at without running
# it. Run from the repository root, after wasm-bindgen wrote playground/pkg. CHROME names another
# Chrome binary.
set -euo pipefail
out=${1:?usage: playground/screenshots.sh DIR}
mkdir -p "$out"
port=$((20000 + RANDOM % 20000))
python3 -m http.server "$port" --bind 127.0.0.1 --directory playground >/dev/null 2>&1 &
server=$!
trap 'kill "$server"' EXIT
sleep 1
for size in desktop:1440,900 phone:390,900; do
    for scheme in light dark; do
        dark=()
        [ "$scheme" = dark ] && dark=(--force-dark-mode)
        "${CHROME:-google-chrome}" --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
            --virtual-time-budget=10000 "${dark[@]}" --window-size="${size#*:}" \
            --screenshot="$out/${size%%:*}-$scheme.png" "http://127.0.0.1:$port/" 2>/dev/null
    done
done
ls "$out"
