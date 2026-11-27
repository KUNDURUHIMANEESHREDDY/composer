# External-capability-first composition policy

Composer is a **capability assembler**, not a substitute implementation generator. The capabilities assembled into deliverables should originate in independently maintained, identifiable external software, not in source files freshly invented by Composer or copied from this project's own fixtures.

## Prioritized sources

1. Prefer a maintained upstream package when its interface, license and dependency versions fit.
2. Otherwise select a pinned external Git repository and extract the smallest complete reusable module/capability with its supporting files.
3. When incompatible, use minimal reviewable **glue or API adapters** around the actual upstream implementation; do not silently replace it with a newly authored algorithm.
4. If a capability cannot be safely reused, **report blocked / unsupported**. Never call generated alternative behavior an imported capability.

Writing integration tests, type adapters, configuration, and orchestration is allowed. Writing a replacement domain implementation and presenting it as sourced from another repository is not.

## Admission and evidence

- Record the upstream repository URL, immutable commit SHA, selected source files and actual upstream license/notice documents. The owner must not be the current Composer owner for an external-source acceptance benchmark.
- Confirm the source files and dependencies form a complete, auditable boundary. Unresolved imports, unsafe symbolic assumptions or uncertain licensing are hard review gates, not reasons to relax verification.
- Preserve original source and target checkouts. Work in disposable target copies.
- Compile and execute in an isolated, network-disabled runner with bounded resources. Docker is not hostile-code isolation, and a microVM or equivalently hardened worker is required before running untrusted third-party builds at scale.
- Require independently specified **behavioral assertions**, including edge cases and wrong-but-compilable counterexamples. A Cargo build alone is not a capability success.
- Report source acquisition, dependency solving, compiler success, behavioral correctness and repeatability separately. Rejections must not be counted as successful integrations.
- Retain upstream provenance and license material in the published output.

## Current implementation boundary

The existing Rust integration flow implements reviewed source/capsule reuse and conservative direct Cargo dependency merging, not fully automated package selection or general repository-to-repository repair. External library discovery and reuse must be measured rather than assumed. The attached held-out Luhn test exercises real source reuse without synthesizing the Luhn algorithm. Package-first autonomous selection and complex dependency reconciliation remain engineering milestones.
