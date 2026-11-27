# Held-out 30-repository audit — scope and honesty requirements

## Dataset and execution

`tests/heldout30_pins.json` freezes 30 independent public Rust repositories at exact Git
commits on 27 November 2026. They are disjoint from the repositories used by
the original integration acceptance tests. The list was chosen **before**
Composer was run against these checkouts. A proposed project URL that did not
resolve was corrected before freezing the cohort.

`.github/workflows/heldout30-static.yml` runs three shards of ten on
GitHub-hosted Linux, using the **existing Composer Rust CLI**, not a new
parser. Every project is fetched by immutable SHA with no submodules;
each has its checked-out commit/tree independently read from Git. Composer's
`analyze` is run twice against the same files. Pass requires >0 source files
and symbols, deterministic JSON, files corresponding to Git-tracked paths,
no traversal escapes, and an unchanged source checkout.

The test records per-project failure classifications, exact Git SHA/tree,
graph SHA256, source/symbol counts, and elapsed time. Failures remain in the
denominator and cause the shard to fail; per-repository JSON results are kept
as Actions artifacts even when a shard fails. No upstream Cargo build, tests,
dependency install, or project-controlled code is executed by this audit.

## Interpretation

This is a **static source-analysis compatibility study**. It does not test
Composer's capability selection, cross-repository semantic matching, API
adaptation, compiler/linker behavior, sandbox isolation, or target functional
contracts. Therefore it is **not** the requested 30-repository functional
integration benchmark and cannot justify general compatibility or a
production-ready label, regardless of how many static cases pass.

For each of the same 30 targets, functional release acceptance will require:
an independently specified target-owned behavior oracle, a documented
appropriate external capability or an honest unsupported rejection, a
real offline isolated integration run, a type-correct wrong-behavior negative
control, exact source and lockfile provenance, original-target preservation,
and at least three fresh independent repetitions. Count eligible successes
and failures separately from unsupported repositories and never silently
discard an unsuccessful attempt.

No production release or automatic merge is authorized by this audit.
Windows Docker Desktop `runsc` execution, independent threat-model and
supply-chain review, code-signing/updates, and clean-machine user acceptance
remain separate release gates.
