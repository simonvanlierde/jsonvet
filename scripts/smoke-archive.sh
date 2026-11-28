#!/usr/bin/env bash
# Unpack each x86-64 Linux release archive in DIR, check it holds what the release promises,
# and run the binary it holds.
#
# Usage: scripts/smoke-archive.sh DIR VERSION
#
# The other targets need their own runner. A missing file, a wrong version, a binary that
# accepts a duplicate key or a JSON report that is not schema_version 2 exits 1.
set -euo pipefail

dir=$1
version=${2#v}
work=$(mktemp -d "${TMPDIR:-/tmp}/smoke-archive.XXXXXX")
trap 'rm -rf "$work"' EXIT
printf '{"a": 1, "a": 2}\n' >"$work/dup.json"

fail() {
  echo "::error::$*"
  exit 1
}

for target in x86_64-unknown-linux-gnu x86_64-unknown-linux-musl; do
  unpacked=$work/$target
  mkdir "$unpacked"
  tar -xzf "$dir/jsonvet-$target.tar.gz" -C "$unpacked"
  # Exactly these files: an extra one is as unplanned as a missing one.
  found=$(cd "$unpacked" && LC_ALL=C ls)
  [[ "$found" == $'LICENSE-APACHE\nLICENSE-MIT\njsonvet' ]] || fail "$target archive holds: $found"
  test -x "$unpacked/jsonvet" || fail "$target jsonvet is not executable"
  got=$("$unpacked/jsonvet" --version)
  [[ "$got" == "jsonvet $version" ]] || fail "$target reports '$got', not $version"
  # Exit 1 is a problem found; 2 would be a broken binary.
  status=0
  report=$("$unpacked/jsonvet" --format json "$work/dup.json") || status=$?
  [[ $status -eq 1 ]] || fail "$target exited $status on a duplicate key"
  [[ "$report" == '{"schema_version":2,'* ]] || fail "$target JSON report lacks schema_version 2"
  echo "$target ok"
done
