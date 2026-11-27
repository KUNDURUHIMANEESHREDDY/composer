# Read-only Rust API discovery — pinned public-source validation

Observed on GitHub Actions (UTC 2026-11-27).

- Successful 5-source benchmark: https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45933595453
- Successful Rust formatting/Clippy/unit/Docker check at the same implementation revision: https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45933602683
- Reproducible script: tests/test_rust_api_discovery_public.py
- CLI: composer suggest-rust-api <verified_capsule> <existing_target> [--output fresh-report.json]

## Pinning and license context

All test repositories were checked out at exact Git revisions. No third-party Rust source was executed by the discovery command; only Composer's trusted own Rust CLI was built and run. Test targets are **disposable copies** of the real existing library repositories with explicit unimplemented public API contracts appended for the benchmark. This does NOT establish real production demand for those APIs or grant legal permission to reuse the source.

| Source repository | Exact commit | License evidence |
| --- | --- | --- |
| TheAlgorithms/Rust | 976f1cdcdab3fab85301c38b28c28e6c5c503e9a | LICENSE (MIT) |
| EbTech/rust-algorithms | 2073670c540935960849f365e1895f558fa4319d | LICENSE (MIT) |
| rust-lang/rustc-hash | fdfe9e6ebea0f0bd3de95d5af849f5d03f4b7fbb | LICENSE-MIT (MIT or Apache) |
| rust-lang/glob | 057cef2824f5a96f738e52d7d8ca3e024381bcc6 | LICENSE-MIT (MIT or Apache) |
| rust-lang/rustc-demangle | 4023665a9bafb7cd9808033fd76fa50f83b9542c | LICENSE-MIT (MIT or Apache) |

## Independent results

| Source -> real target copy | Contract and outcome |
| --- | --- |
| TheAlgorithms euler_totient (u64 -> u64) -> EbTech | PASS: detects unfinished adapted_totient(u32) -> u128 and recommends euler_totient=adapted_totient(u32)->u128. Safe fixed-width unsigned widening; deterministic name-token overlap |
| EbTech fast_gcd (mut i64, mut i64 -> i64) -> TheAlgorithms | PASS: detects unfinished fast_gcd_checked(i32,i32)->i64 and recommends fast_gcd=fast_gcd_checked(i32,i32)->i64. Static parser accounts for simple mut bindings; signed widening |
| TheAlgorithms euler_totient -> EbTech unrelated_password | NO MATCH: unrelated name; no unsupported recommendation |
| EbTech fast_gcd -> TheAlgorithms fast_gcd_narrow(i64,i64)->i32 | NO MATCH: unsafe return narrowing (i64 to i32) |
| Either positive source -> already implemented target function | IGNORED: discovery only examines exact todo!()/unimplemented!() root-level library stubs |
| rustc-hash FxHasher three-file capsule -> EbTech | NO MATCH: no suitable scalar public API contract |
| rust-lang/glob public glob capsule -> EbTech | NO MATCH: custom Result/Paths API not supported by bounded primitive adapter |
| rust-lang/rustc-demangle Demangle three-file capsule -> EbTech | NO MATCH: opaque/generic API not representable in supported scalar signature contract |

The benchmark asserts the original target library is byte-identical before/after suggestion generation, and source repos are never mutated. Incompatible cases are **rejections**, not transferred features or successful compilations. A successful suggestion is not a successful task execution.

## Safety and inference boundaries

The command first runs Feature Capsule hash validation and parses selected Rust source without executing it. It reads target src/lib.rs only if it is a regular, nonsymlink file under a 1 MiB cap. It limits selected source to 4 MiB, caps public functions at 256, and returns up to five ranked candidates per requirement. It only considers directly exported top-level plain functions with supported primitive/string signatures, maximum 4 parameters, and fixed-width infallible integer widening (plus strictly admitted identities / &str to String). Generic, trait, impl-method, async/unsafe, complex type, and unsupported signatures are skipped; name similarity does not establish semantic intent.

All suggestions are status-labeled **requires independent review and compiler verification**. Ties are not automatically chosen. To execute a reviewed suggestion, the caller must separately use composer verify-rust with --api-signature and license/source review, then pass offline compiler and independent behavioral contract tests. No target/source files are rewritten by the discovery command.

## Remaining open gates

- More positive unfamiliar-source transfer benchmarks; two positive **suggestions** across two different projects do not measure general reliability
- Independent dynamic verification of each suggested mapping (existing compiler adapter supports this separately, but not autonomously triggered)
- Structured API requirements outside explicit unfinished target functions, traits/methods/generics, custom types and complex ownership
- Semantic equivalence, lifecycle/errors, broad Cargo features/workspaces, security reviews of third-party code and production-grade isolation
- Native Windows build of this new branch and clean-machine installer are not shown by GitHub's Linux CI
