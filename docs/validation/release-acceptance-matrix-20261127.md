# Composer release acceptance matrix — 27 November 2026

This report distinguishes **observed test results** from expectations. A green
workflow proves only the assertions its jobs actually perform. No claim of
complete supply-chain security or arbitrary cross-repository composition is
authorized by this document.

| Area | Current evidence | Release decision |
| --- | --- | --- |
| Integrated main candidate | PR #34 merge commit `1d3d323`: 15/15 workflows passed, including Linux gVisor, locked native Windows Rust build, RustSec, NSIS install/uninstall, and GitHub build attestations | Integrated baseline verified; follow-on commits require independent repeat |
| Previously unseen static analysis | `875c09c`: held-out 30/30 Git-pinned Rust repos, identical two-run analysis, untouched originals, 3 shards; [Actions evidence](https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45992440098) | STATIC inspection pass only |
| Static scope size | 3,062 files parsed; 56,129 symbols reported. Individual case elapsed includes checkout + two analyses: p50 0.604s, p95 2.060s, max 2.518s on GitHub-hosted Linux; not an installed-Windows benchmark | Supporting diagnostics, not a production performance SLA |
| Held-out functional integration | `dtolnay/anyhow` passed **3/3 fresh full-target isolated verification runs**, with an independent totient oracle; deliberately wrong factorial rejected without publishing output. [Evidence workflow](https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45992883550). Three other frozen targets are newly scheduled for their own acceptance workflows. | **1/30 functional targets demonstrated**; 29/30 remain unproven until independent results |
| Broad behavioral compatibility | Only a few curated external capability/target tests have passed. Zero behavioral integrations were attempted in the 30-repo static workflow | **Not proven**; pre-register 30 functional contracts or honest unsupported cases, count all failures, repeat three times |
| Host Git execution trust boundary | Effective checkout config scanned for clean/smudge/process helpers and fsmonitor disabled; newly added cross-platform tests; source-acquisition host Git now excludes inherited config overrides and templates | Requires exact-head tests and independent hostile-input review |
| Dependency and pipeline supply chain | RustSec advisory scan, pinned GitHub Actions, Syft SPDX SBOM, Cargo Git commit/tree/blob verification, GitHub Sigstore-backed build attestation | Useful controls; full review of upstream signer identities, SBOM completeness, runtime and build infrastructure still open |
| Windows installer | GitHub hosted Windows NSIS install, CLI smoke, uninstall and unrelated-file-preservation acceptance passed | Not Authenticode-signed; actual distribution/update/rollback and independent review open |
| Linux runsc | Existing Linux gVisor CI run passed | Not a proof of complete sandbox escape resistance |
| Windows Docker Desktop runsc | Connected host: Docker CLI available, Linux daemon absent, `runsc` not verified, ~1 GiB free RAM at check | **Blocked**; do not kill user programs or silently downgrade runtime |
| Repository release controls | GitHub REST returned no rulesets; classic branch-protection API returned 403 for this connection | Protection unverified; release owner must independently verify required reviews/CI before merging |
| External security approval | No independent reviewer sign-off or penetration/security acceptance report | **Blocked** |
| Production readiness | Incomplete functional evidence, independent security approval, local Windows runsc, actual signed release/update/rollback | **Blocked: no production designation** |

Open audit: [issue #35](https://github.com/KUNDURUHIMANEESHREDDY/composer/issues/35).
The draft PR remains separate from `main` and must not be merged simply because
a smaller subset of unrelated CI workflows passes.
