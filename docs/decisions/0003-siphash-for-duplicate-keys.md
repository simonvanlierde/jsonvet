# 3. SipHash with OS keys for the duplicate-key index

Status: accepted (2026-10-20)

## Context

To find duplicate keys, the parser compares an object's keys by a scan up to 16 keys, then
through a hash index. A hash table is linear only while keys do not collide. With a fixed or
weak hash function, a file built to collide its keys makes the check quadratic, and
[SECURITY.md](../../SECURITY.md) promises that a crafted file cannot use far more time than its
size explains.

The first design, from a survey of 2026-10-13, picked FxHash, on the argument that hash flooding
does not matter for a local checker. It does matter here: jsonvet runs in CI on files from
pull requests.

## Options

- **FxHash.** Fast, but its keys are fixed, so colliding keys can be computed offline. Not
  measured; turned down on that argument.
- **foldhash**, hashbrown's default. Measured on 2026-10-20 on one Linux machine, swapping the
  index's hasher:

  | Input | SipHash | foldhash |
  | --- | ---: | ---: |
  | one object of 2,000,000 keys (47 MB) | 388 ± 17 ms | 350 ± 18 ms |
  | 500,000 small objects | 134 ms | 132 ms (within σ) |

  About 10% on a huge object, nothing on the common shape. foldhash is seeded per process, but it
  is not built to resist keys crafted to collide, and SipHash is.
- **The standard library's SipHash, keyed from the OS** at each start.

## Decision

Keep SipHash with OS keys. A gain that shows only on objects of millions of keys does not pay
for weaker resistance to crafted files.

## Consequences

- The check stays linear in the file's size on any input, as SECURITY.md promises.
- Instruction counts move by under 0.3% between runs, because the keys are random. CI's
  instruction-count gate fails only past 1%.
- One mutant in `Keys::first_or_insert` cannot be caught: only keys whose 32-bit hashes collide
  tell it apart from the real code, and no test can pick such a pair.
  `.cargo/mutants.toml` lists it.

## Revisit if

- real files with objects of hundreds of thousands of keys become a common workload, and a keyed
  hasher with SipHash's flood resistance measures past 5% faster on them.
