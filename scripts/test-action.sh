#!/usr/bin/env bash
# Tests the GitHub Action's logic in scripts/action.sh, without installing anything. CI runs it
# with /bin/bash, which is bash 3.2 on macOS: `bash scripts/test-action.sh`.
set -o pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=scripts/action.sh
. "$here/action.sh"

failures=0
fail() {
  echo "FAIL: $*"
  failures=$((failures + 1))
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
printf '[workspace.package]\nversion = "9.8.7"\n' > "$tmp/Cargo.toml"

# version input, action ref, expected specifier
while IFS='|' read -r version ref want; do
  got=$(resolve_spec "$version" "$ref" "$tmp/Cargo.toml" 2>/dev/null) || fail "resolve_spec '$version' '$ref' failed"
  [[ "$got" == "$want" ]] || fail "resolve_spec '$version' '$ref': got '$got', want '$want'"
done <<'EOF'
|v3|==3.*
|v3.0|==3.0.*
|v3.0.0|==3.0.0
|0123456789abcdef0123456789abcdef01234567|==9.8.7
|main|==9.8.7
||==9.8.7
|v3.0.0-rc.1|==9.8.7
latest|v3|
3.0.0|v3|==3.0.0
3.0.0a1|v3|==3.0.0a1
3.0.0b2|v3|==3.0.0b2
3.0.0rc1|v3|==3.0.0rc1
3.0.0.post1|main|==3.0.0.post1
3.*|0123456|==3.*
3.0.*|main|==3.0.*
EOF
for version in '>=3' '3.0' '3' 'v3.0.0' '3.0.0 ' '==3.0.0' 'latest;' '3.0.0rc' '3.0.0.post' '3.0.0dev1' '3.0.0-rc.1'; do
  resolve_spec "$version" v3 "$tmp/Cargo.toml" > /dev/null 2>&1 && fail "resolve_spec accepted '$version'"
done
resolve_spec "" main "$tmp/missing.toml" > /dev/null 2>&1 && fail "resolve_spec passed without a Cargo.toml"

# The default list: tracked files the hook would check, at any depth, without symlinks or
# submodules.
repo="$tmp/repo"
mkdir -p "$repo/nested/deep" "$repo/sub" "$tmp/outside"
echo '{}' > "$tmp/outside/c.json"
(
  cd "$repo" || exit 1
  git init -q .
  echo '{}' > a.json
  echo '{}' > nested/deep/b.jsonc
  echo '{}' > .babelrc
  echo '{}' > sub/.babelrc
  echo 'x' > notes.txt
  ln -s a.json link.json
  echo '{}' > untracked.json
  # Untracked links for the files input: one back up the tree, one out of the repository.
  ln -s . loop
  ln -s ../outside out
  git add .babelrc a.json nested sub notes.txt link.json
  git update-index --add --cacheinfo "160000,0123456789abcdef0123456789abcdef01234567,mod.json"
)
want=$'.babelrc\na.json\nnested/deep/b.jsonc\nsub/.babelrc'
got=$(cd "$repo" && list_files "" python3 && printf '%s\n' "${files[@]}")
[[ "$got" == "$want" ]] || fail "default list: got [$got], want [$want]"

# The files input: `**` reaches any depth (bash 3.2 has no globstar) but through no symlink, a
# name that matches nothing stays, a file named twice is checked once, and two spaces make no
# empty name.
got=$(cd "$repo" && list_files "**/*.json  **/b.jsonc missing*.json untracked.json" python3 && printf '%s\n' "${files[@]}")
want=$'a.json\nuntracked.json\nnested/deep/b.jsonc\nmissing*.json'
[[ "$got" == "$want" ]] || fail "files input: got [$got], want [$want]"
# A symlinked directory named in a pattern is not entered either; the pattern stays as given.
# A symlink as the last segment is a file name, kept.
got=$(cd "$repo" && list_files "out/*.json link.json" python3 && printf '%s\n' "${files[@]}")
want=$'out/*.json\nlink.json'
[[ "$got" == "$want" ]] || fail "symlinked directory: got [$got], want [$want]"

# SARIF: the output must be a SARIF 2.1.0 document, whatever the exit code says.
fake() {
  printf '#!/usr/bin/env bash\n%s\n' "$1" > "$tmp/jsonvet"
  chmod +x "$tmp/jsonvet"
}
fake 'echo "{\"version\": \"2.1.0\", \"runs\": []}"; exit 1'
write_sarif "$tmp/jsonvet" python3 "$tmp/out.sarif" -- a.json 2>/dev/null || fail "SARIF with problems was rejected"
fake 'echo "{\"version\": \"2.1.0\", \"runs\": []}"'
write_sarif "$tmp/jsonvet" python3 "$tmp/out.sarif" -- a.json 2>/dev/null || fail "clean SARIF was rejected"
# jsonvet 2.0 takes `--format sarif` for two file names it cannot read.
fake 'echo "--format: cannot read"; exit 1'
write_sarif "$tmp/jsonvet" python3 "$tmp/out.sarif" -- a.json 2>/dev/null && fail "text output passed as SARIF"
fake 'echo "[1]"; exit 1'
write_sarif "$tmp/jsonvet" python3 "$tmp/out.sarif" -- a.json 2>/dev/null && fail "a JSON array passed as SARIF"
fake 'echo "{\"version\": \"2.1.0\"}"; exit 2'
write_sarif "$tmp/jsonvet" python3 "$tmp/out.sarif" -- a.json 2>/dev/null && fail "exit 2 passed"
write_sarif "$tmp/jsonvet" python3 "$tmp/no/such/dir/out.sarif" -- a.json 2>/dev/null && fail "an unwritable path passed"

if [[ $failures -gt 0 ]]; then
  echo "$failures failed"
  exit 1
fi
echo "all action tests passed (bash $BASH_VERSION)"
