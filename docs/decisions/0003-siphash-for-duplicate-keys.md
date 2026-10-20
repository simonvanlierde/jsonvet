# 3. SipHash with OS keys for the duplicate-key index

Status: accepted (2026-10-20)

## Context

An object's keys are compared by a scan up to 16 keys, then through a hash index. With a weak or
fixed hash, a file built to collide its keys makes that quadratic, and
[SECURITY.md](../../SECURITY.md) promises a crafted file costs about what its size explains. The
first design picked FxHash, arguing hash flooding does not matter for a local checker. It does:
jsonvet runs in CI on pull requests.

## Options

- **FxHash.** Fixed keys, so colliding keys can be computed offline. Argued, not measured.
- **foldhash**, hashbrown's default. Swapped in on 2026-10-20 on one Linux machine: one object of
  2,000,000 keys (47 MB) went from 388 ± 17 to 350 ± 18 ms; 500,000 small objects stayed within σ
  (134 against 132 ms). It is seeded per process but not built to resist crafted collisions.
- **The standard library's SipHash, keyed from the OS** at each start.

## Decision

Keep SipHash. About 10% on objects of millions of keys does not pay for weaker resistance to
crafted files.

## Consequences

- The check stays linear on any input.
- Instruction counts move by under 0.3% between runs, since the keys are random; CI's
  instruction-count gate fails only past 1%.
- One mutant in `Keys::first_or_insert` cannot be killed: only two keys with colliding 32-bit
  hashes tell it apart, and no test can pick them. `.cargo/mutants.toml` lists it.

## Revisit if

- objects of hundreds of thousands of keys become common, and a keyed hasher as flood-resistant
  as SipHash measures past 5% faster on them.
