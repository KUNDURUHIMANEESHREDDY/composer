# Recovery from a real third-party nightly-only benchmark failure

Previous attempted integration into pinned public Rust target
`rapidfuzz/strsim-rs` (`dacc84c0dc61eff0ee0ff66962bcf2e17018ad26`)
failed even though the intended imported capability was unrelated to its
benchmarks. The target has `benches/benches.rs` with
`#![feature(test)]`, which requires nightly Rust. Composer deliberately uses
stable Rust inside an offline Docker sandbox and must not silently switch to
nightly, turn off resource limits, or claim an all-targets pass.

## Focused, explicitly weaker verification scope

The existing `verify-external-git-rust` command accepts
`--contract-focused` to select Cargo's existing
`cargo test --locked --offline --lib --tests`. This exercises the
third-party target's library and integration tests, **including the mandatory
independent target-owned contract test**. It excludes benchmarks, examples
and binary test targets. The default command (without the flag) still runs
`cargo test --locked --offline --all-targets`, with no automatic fallback.

The JSON result labels `verification_scope =
library_and_integration_tests`. The persisted evidence additionally records
`third_party_benchmarks_verified = false`. The recovered target is
NOT described as fully compatible with all targets and should never be used
as production acceptance without separate benchmark/binary validation.

## Regression gate

`tests/test_recovered_nightly_bench_target.py` clones the exact failing
repository, uses the real existing external `TheAlgorithms/Rust`
Euler-totient implementation via Cargo's Git dependency resolver, and:

- proves original all-targets mode still rejects the pinned nightly-only
  benchmark, leaving the original unchanged;
- uses the explicit scoped mode on **three fresh target copies**, requiring
  independent Euler-totient oracle tests, exact pinned Git revision, source
  Git-object evidence, retained license, and offline Docker execution;
- deliberately imports the wrong but type-compatible factorial function and
  requires semantic rejection and no published output.

This tests a previously failing unrelated GitHub target rather than adding
another curated happy-path success. The second unrelated target repository
was already verified separately in PR #28. These small samples are not proof
of broad autonomous API inference or reliability across arbitrary GitHub
projects. Resource-limited `sleep_sort` suites, offline Cargo version
conflicts, and full target verification remain distinct known blockers.
