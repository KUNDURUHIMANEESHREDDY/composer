# Initial Threat Model

## Assets

- User source projects and generated target projects.
- Repository snapshots and extracted source code.
- Provider credentials and model API keys.
- Build artifacts, logs, and provenance records.
- Workflow state and project specifications.

## Trust boundaries

1. User/API boundary.
2. External repository and package-provider boundary.
3. Agent reasoning boundary.
4. Control-plane to execution-worker boundary.
5. Worker to artifact-store boundary.

## Untrusted inputs

Treat repository contents, README files, comments, build files, package scripts, dependency metadata, generated code, and test output as untrusted data. Repository text must never override agent instructions or security policies.

## Primary threats

- Malicious build scripts or package lifecycle hooks.
- Credential theft from environment variables or mounted files.
- Network access to internal services or cloud metadata endpoints.
- Path traversal and symlink attacks during extraction.
- Resource exhaustion through large repositories or tests.
- Dependency confusion and compromised package releases.
- Prompt injection embedded in repository content.
- License misclassification or missing attribution.
- Provenance records that do not match the delivered artifact.

## Initial controls

- Do not execute code during repository acquisition or static analysis.
- Use ephemeral execution environments for builds and tests.
- Default-deny outbound network access in build workers.
- Do not place provider credentials in untrusted execution workers.
- Apply CPU, memory, process-count, disk, and time limits.
- Reject archive and extraction paths escaping the workspace.
- Pin repositories to immutable commit IDs.
- Hash source snapshots and delivered artifacts.
- Preserve license files and notices for reused code.
- Record the evidence behind security and licensing decisions.
- Verify each capsule-listed path, file type, byte size, and SHA-256 digest before compatibility analysis or staging.
- Reject traversal paths, symlink components, duplicate normalized paths, and pre-existing integration destinations.
- Copy only manifest-listed capsule files into a fresh target-local `.composer/integrations/` namespace; never overwrite target source.
- Re-verify the staged capsule before atomically publishing the staging directory.
- Do not install dependencies or invoke package lifecycle hooks, build scripts, tests, or source code during compatibility analysis/staging.
- Treat target manifests, dependency constraints, symbol collision reports, and SPDX text as untrusted evidence; they are not resolver, semantic-equivalence, authenticity, or legal verdicts.

## Open risks

Containers share the host kernel. Production execution of hostile code may require stronger isolation such as microVMs, separate worker hosts, and carefully constrained network proxies. Static analysis cannot prove the absence of runtime dependencies or malicious behavior.
