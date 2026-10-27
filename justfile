# Developer tasks. Each recipe runs the command CI runs for the same step (.github/workflows/).
# Tool versions: mise.toml.

# Where cargo puts its output, for the recipes that run the release binary.
target := env("CARGO_TARGET_DIR", "target")

default:
    @just --list

# Once per clone: the json5-tests submodule and the git hooks (commit and push).
setup:
    git submodule update --init
    prek install

# ci.yml `lint`.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
    cargo deny check --hide-inclusion-graph
    prek run ruff-check ruff-format zizmor --all-files --show-diff-on-failure
    uv run scripts/check-versions.py
    just matrix-check
    uv run conformance/run.py --check

# ci.yml `test`. nextest skips doctests, so they run on their own.
test:
    cargo nextest run --workspace --locked
    cargo test --workspace --doc --locked

# ci.yml `scaling`: code paths that could turn quadratic stay linear, by instruction counts
# (needs valgrind, so Linux).
scaling:
    cargo build --release --manifest-path fuzz/Cargo.toml --bin diff_json5
    JSONVET_DIFF_JSON5={{ absolute_path(env("CARGO_TARGET_DIR", "fuzz/target")) }}/release/diff_json5 \
        cargo test --release --locked -p jsonvet-cli --test scaling -- --ignored

# ci.yml `bench`: instruction counts for `check` and `--schema` (needs valgrind and gungraun-runner,
# so Linux). ARGS go to gungraun, such as --save-baseline=base or --baseline=base.
counts *ARGS:
    cargo bench --locked -p jsonvet --bench check -- {{ ARGS }}
    cargo bench --locked -p jsonvet-cli --bench schema -- {{ ARGS }}

# ci.yml `differential`: jsonvet against check-json5 1.4.1 on generated files (CI samples 5,000).
differential CASES="5000" SEED="7":
    cargo build --release --locked -p jsonvet-cli
    uv run scripts/differential.py --cases {{ CASES }} --seed {{ SEED }} --jsonvet {{ target }}/release/jsonvet

# ci.yml `lint` and `test` on this OS. CONTRIBUTING.md lists the CI jobs it leaves out.
check: lint test

# ci.yml `msrv`: the oldest Rust the crates support (`rust-version` in Cargo.toml).
msrv:
    cargo +1.88 check --workspace --lib --bins --locked

# Re-run the conformance matrix (needs docker), then render docs/conformance.md and the README block.
matrix:
    cargo build --release -p jsonvet-cli
    uv run conformance/run.py --jsonvet {{ target }}/release/jsonvet
    uv run conformance/report.py

# Refresh the matrix's lock files after you change a parser pin, then run `just matrix`.
matrix-lock:
    uv run conformance/run.py --lock

# ci.yml `lint`: the rendered page is up to date. Fails on uncommitted edits to those two files.
matrix-check:
    uv run conformance/report.py
    git diff --exit-code docs/conformance.md README.md

# Fuzz one target for 20 minutes, as fuzz.yml does: check, diff_serde_json, diff_json5 or parse.
fuzz TARGET:
    cd fuzz && cargo +nightly fuzz run -O {{ TARGET }} -- -max_total_time=1200 -print_final_stats=1

# Mutation testing over the crates in one go, which takes over an hour; mutants.yml splits it
# into ten shards. The 8 GiB cap, as in mutants.yml, makes a mutant that allocates without end
# fail its test instead of taking the machine's memory.
mutants:
    ulimit -v 8388608 && cargo mutants --workspace --no-shuffle

# Time jsonvet against check-json5 1.4.1 (needs hyperfine).
bench *ARGS:
    cargo build --release -p jsonvet-cli
    uv run scripts/bench.py --jsonvet {{ target }}/release/jsonvet {{ ARGS }}

# Build the playground as pages.yml does, then serve it at http://127.0.0.1:8000. Needs the
# wasm32 target and wasm-bindgen-cli at the version in Cargo.lock; the deploy also runs wasm-opt.
# Runs the page's tests and smoke test before serving, as pages.yml does.
playground PORT="8000":
    #!/usr/bin/env bash
    set -euo pipefail
    want=$(cargo pkgid --locked wasm-bindgen | sed 's/.*@//')
    if [ "$(wasm-bindgen --version 2>/dev/null | cut -d' ' -f2)" != "$want" ]; then
        echo "needs wasm-bindgen-cli $want: cargo install wasm-bindgen-cli --version $want --locked" >&2
        exit 1
    fi
    node --test playground/lines.test.mjs
    cargo build --locked -p jsonvet-wasm --target wasm32-unknown-unknown --profile wasm
    wasm-bindgen --target web --no-typescript --out-dir playground/pkg \
        {{ target }}/wasm32-unknown-unknown/wasm/jsonvet_wasm.wasm
    node playground/smoke.mjs
    python3 -m http.server {{ PORT }} --bind 127.0.0.1 --directory playground
