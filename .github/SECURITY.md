# Security Policy

Composer is under active development. **No production release is currently security-certified or supported.**

## Reporting a vulnerability

Please do **not** disclose exploitable details, secrets, credentials, or attack reproductions in public GitHub issues or pull requests.

Use GitHub's private vulnerability reporting for this repository (**Security → Advisories → Report a vulnerability**) if the option is available. If it is not enabled, ask the repository maintainer for a private reporting channel *without posting the sensitive details publicly*.

Include the affected commit or version, impact, relevant environment and a minimal reproduction when communicating through a private channel. Do not include real credentials or sensitive user data.

The maintainers should acknowledge and triage reports privately, reproduce in isolated test environments, prepare a fix with regression coverage and coordinate disclosure after remediation. No response-time guarantee or independent audit approval is implied by this policy.

## Security boundaries

Composer may inspect and integrate untrusted third-party repositories. Treat third-party source, build scripts, Cargo dependency metadata, hooks and generated outputs as untrusted. Keep sandbox isolation, source verification and independent functional acceptance enabled.

Automated scans, gVisor Linux tests, SBOMs and build provenance do **not** replace a third-party human security review. See [release security blockers](https://github.com/KUNDURUHIMANEESHREDDY/composer/issues/35) before distributing binaries.
