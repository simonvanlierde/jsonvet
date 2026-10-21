# 3. SipHash with OS keys for the duplicate-key index

Status: accepted (2026-10-20)

## Context

Past 16 keys, an object's keys go through a hash index. With a weak or fixed hash, a file built
to collide its keys makes that quadratic, and jsonvet runs in CI on pull requests.

## Options

- **FxHash.** Fixed keys, so collisions can be computed offline. Argued, not measured.
- **foldhash.** On 2026-10-20, one object of 2,000,000 keys went from 388 ± 17 to 350 ± 18 ms;
  500,000 small objects stayed within σ. It isn't built to resist crafted collisions.
- **The standard library's SipHash, keyed from the OS** at each start.

## Decision

Keep SipHash. About 10% on objects of millions of keys doesn't pay for weaker resistance.

## Consequences

The check stays linear on any input. Instruction counts vary by under 0.3% between runs, inside
CI's 1% gate. One mutant in `Keys::first_or_insert` can't be killed, since only two keys with
colliding hashes tell it apart; `.cargo/mutants.toml` lists it.

## Confirmation

Review of `short_hash` in `parse.rs`, and the `scaling` CI job for ordinary keys.

## Revisit if

Objects of hundreds of thousands of keys become common, and a hasher as flood-resistant as
SipHash measures past 5% faster on them.
