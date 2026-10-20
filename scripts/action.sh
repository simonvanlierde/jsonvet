#!/usr/bin/env bash
# The jsonvet GitHub Action's step, run by action.yml with the inputs in FILES, ARGS, SARIF and
# VERSION. scripts/test-action.sh sources this file to test the functions one by one.
#
# It runs on Linux, macOS and Windows runners, and macOS may run bash 3.2: no `set -u` (bash 3.2
# calls an empty array unbound), no globstar, no mapfile.
set -o pipefail

error() {
  echo "::error title=jsonvet action::$*" >&2
}

# Prints the pip version specifier for the jsonvet to install, empty for the newest release.
# Arguments: the `version` input, the action's ref, the action's Cargo.toml.
resolve_spec() {
  local version="$1" ref="$2" cargo_toml="$3"
  local exact='^[0-9]+\.[0-9]+\.[0-9]+((a|b|rc)[0-9]+|\.post[0-9]+)?$' series='^[0-9]+(\.[0-9]+)?\.\*$' tag='^v[0-9]+(\.[0-9]+){0,2}$'
  if [[ -n "$version" ]]; then
    if [[ "$version" == latest ]]; then
      echo ""
    elif [[ "$version" =~ $exact || "$version" =~ $series ]]; then
      echo "==$version"
    else
      error "version must be X.Y.Z, X.Y.ZrcN, X.Y.Z.postN, X.*, X.Y.* or latest, not '$version'"
      return 2
    fi
  elif [[ "$ref" =~ $tag ]]; then
    version="${ref#v}"
    local dots="${version//[^.]/}"
    # A full version pins it; a major or minor tag takes the newest release under it.
    if [[ ${#dots} -eq 2 ]]; then echo "==$version"; else echo "==$version.*"; fi
  else
    # A commit SHA or a branch: the version in that commit's Cargo.toml, which between
    # releases is the last release, not the code in that commit.
    version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$cargo_toml" | head -n 1)
    if [[ -z "$version" ]]; then
      error "found no version in $cargo_toml; set the version input"
      return 2
    fi
    echo "::notice title=jsonvet action::${ref:-this action} is not a release tag, so it installs jsonvet $version, the release named in its Cargo.toml" >&2
    echo "==$version"
  fi
}

# Fills `files` with what to check. Arguments: the `files` input, the Python that expands globs.
list_files() {
  local input="$1" python="$2" file line
  files=()
  if [[ -n "$input" ]]; then
    # Word splitting is the point: the input is a space-separated list. Python expands the globs,
    # since bash 3.2 has no globstar. A pattern that matches nothing stays as it is, so jsonvet
    # reports it as unreadable rather than checking nothing. A file two patterns match is checked
    # once. Wildcards skip symlinks, like the default list: glob.glob follows them, so a link
    # `loop -> .` repeats the tree under `**` and `out -> /` leaves the repository.
    set -f
    # shellcheck disable=SC2206
    local words=($input)
    set +f
    # jsonvet reads a file named `-` as standard input, so it gets a `./`.
    while IFS= read -r -d '' file; do
      [[ "$file" == - ]] && file=./-
      files+=("$file")
    done < <(
      "$python" -I -c '
import glob, os, sys
from fnmatch import fnmatch

def expand(done, parts):
    if not parts:
        if "/".join(done):
            yield "/".join(done)
        return
    head, rest = parts[0], parts[1:]
    if not glob.has_magic(head):
        path = "/".join(done + [head]) or "/"
        # A symlinked directory is not entered, as the wildcards enter none either.
        if os.path.lexists(path) and not (rest and os.path.islink(path)):
            yield from expand(done + [head], rest)
        return
    try:
        entries = list(os.scandir("/".join(done) + "/" if done else "."))
    except OSError:
        return
    if head == "**":
        yield from expand(done, rest)
    for entry in entries:
        if entry.is_symlink() or entry.name.startswith(".") and not head.startswith("."):
            continue
        if head == "**":
            if entry.is_dir():
                yield from expand(done + [entry.name], parts)
            elif not rest:
                yield "/".join(done + [entry.name])
        elif fnmatch(entry.name, head) and (not rest or entry.is_dir()):
            yield from expand(done + [entry.name], rest)

seen = set()
for pattern in sys.argv[1:]:
    # Wildcards match files only: jsonvet cannot check a folder.
    matched = [p for p in sorted(expand([], pattern.replace(os.sep, "/").split("/")))
               if not (glob.has_magic(pattern) and os.path.isdir(p))]
    for path in matched or [pattern]:
        if path not in seen:
            seen.add(path)
            sys.stdout.buffer.write(os.fsencode(path.replace(os.sep, "/")) + b"\0")
' "${words[@]}"
    )
  else
    # The same file names as the pre-commit hook's `files` pattern. A pathspec `*` also matches
    # `/`, so `*.json` finds files at any depth. Only regular files: pre-commit's `types: [text]`
    # skips symlinks (mode 120000) and submodules (160000).
    if ! git rev-parse --is-inside-work-tree > /dev/null 2>&1; then
      error "the default file list needs a git checkout in the working directory: run actions/checkout first, or set the files input"
      return 2
    fi
    while IFS= read -r -d '' line; do
      [[ "$line" == 100* ]] || continue
      files+=("${line#*$'\t'}")
    done < <(
      git ls-files -z -s -- '*.json' '*.jsonc' '*.json5' '*.geojson' \
        '*.jsonld' '*.hujson' '*.jwcc' '*.code-workspace' '*.code-snippets' \
        '*.webmanifest' '*.jsonl' '*.ndjson' '.babelrc' '*/.babelrc'
    )
  fi
}

# Checks the build provenance attestation of a downloaded jsonvet wheel: it must come from this
# repository's release.yml, which runs only for a pushed tag. Releases before 3.1.0 have no
# attestation, so they are installed with a notice. Arguments: the wheel, the `verify` input.
verify_wheel() {
  local wheel="$1" verify="$2" version
  version=$(basename "$wheel" | cut -d - -f 2)
  if [[ "$verify" == false ]]; then
    echo "::notice title=jsonvet action::verify is false, so the jsonvet $version wheel's attestation is not checked" >&2
    return 0
  elif [[ "$verify" != true ]]; then
    error "verify must be true or false, not '$verify'"
    return 2
  fi
  if [[ "$version" =~ ^[0-2]\. || "$version" =~ ^3\.0\. ]]; then
    echo "::notice title=jsonvet action::jsonvet $version has no attestation to check: releases before 3.1.0 have none" >&2
    return 0
  fi
  if ! command -v gh > /dev/null; then
    error "checking the wheel's attestation needs the GitHub CLI (gh), which hosted runners have; install it, or set verify: false"
    return 2
  fi
  if ! gh attestation verify "$wheel" --repo simonvanlierde/jsonvet \
    --signer-workflow simonvanlierde/jsonvet/.github/workflows/release.yml > /dev/null; then
    error "the jsonvet $version wheel from PyPI has no valid attestation from simonvanlierde/jsonvet's release workflow, so it was not installed; set verify: false only if you trust your package index"
    return 2
  fi
}

# Writes the SARIF file. Arguments: the jsonvet binary, the Python, the SARIF path, then the
# jsonvet arguments. Exit 1 means problems found, but jsonvet before 3.0.0 has no `--format` and
# takes it for a file name, and a path that cannot be written also gives 1, so check what landed.
write_sarif() {
  local jsonvet="$1" python="$2" sarif="$3"
  shift 3
  local status=0
  "$jsonvet" --format sarif "$@" > "$sarif" || status=$?
  if [[ $status -gt 1 ]] || ! "$python" -I -c '
import json, sys
try:
    sys.exit(json.load(open(sys.argv[1], encoding="utf-8")).get("version") != "2.1.0")
except (OSError, ValueError, AttributeError):
    sys.exit(1)
' "$sarif"; then
    if [[ $status -gt 2 ]]; then
      error "jsonvet did not write SARIF to $sarif: it could not run (exit $status)"
    else
      error "jsonvet did not write SARIF to $sarif (exit $status); the sarif input needs jsonvet 3.0.0 or later"
    fi
    return 2
  fi
}

# The inputs come from the environment, in capitals.
# shellcheck disable=SC2153
main() {
  local spec
  spec=$(resolve_spec "$VERSION" "$ACTION_REF" "$GITHUB_ACTION_PATH/Cargo.toml") || exit 2

  local venv="$RUNNER_TEMP/jsonvet-venv" python=python3
  # On Windows `python3` can be the Microsoft Store stub.
  [[ "$RUNNER_OS" == Windows ]] && python=python
  # -I keeps the checked-out repository off Python's path, so a json.py or venv/ in it never runs.
  "$python" -I -m venv "$venv" || exit 2
  local bin="$venv/bin"
  [[ -d "$bin" ]] || bin="$venv/Scripts"
  # Downloaded first, so the wheel's attestation is checked before it is installed.
  # Forward slashes: a `\` in a Windows path would escape the next character in the glob.
  local wheels="${RUNNER_TEMP//\\//}/jsonvet-wheel"
  "$bin/python" -I -m pip download -q --disable-pip-version-check --no-deps --only-binary :all: \
    -d "$wheels" "jsonvet$spec" || exit 2
  local found=("$wheels"/jsonvet-*.whl)
  verify_wheel "${found[0]}" "$VERIFY" || exit 2
  # Only gh needs the token; jsonvet reads files from strangers.
  unset GH_TOKEN
  "$bin/python" -I -m pip install -q --disable-pip-version-check --no-deps --no-index "${found[0]}" || exit 2
  "$bin/jsonvet" --version

  set -f
  # shellcheck disable=SC2206
  local args=($ARGS)
  set +f
  list_files "$FILES" "$bin/python" || exit 2
  if [[ ${#files[@]} -eq 0 ]]; then
    echo "::warning title=jsonvet action::found no JSON files to check" >&2
  fi
  # shortcut: one command line for all files; Windows caps it at 32,767 characters, so a
  # repository with thousands of files needs batching there.

  # A failing check must not end the script before the SARIF run; the step still exits with the
  # text run's status.
  local status=0
  "$bin/jsonvet" "${args[@]}" -- "${files[@]}" || status=$?
  if [[ -n "$SARIF" ]]; then
    # A second run, so the log and the annotations come from the text format.
    write_sarif "$bin/jsonvet" "$bin/python" "$SARIF" "${args[@]}" -- "${files[@]}" || exit 2
  fi
  exit "$status"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main
fi
