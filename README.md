# Software Composer

**A Rust-first platform for reusing software capabilities across repositories.**

Software Composer is being built to help developers assemble applications from proven implementations instead of repeatedly rebuilding every capability from scratch. It is designed to discover useful code in existing repositories, understand what that code depends on, extract a capability with its dependency closure, integrate it into a target project, and verify that the result works.

The central idea is **software composition with evidence**: every reused capability should have a traceable source, an explainable dependency boundary, a deliberate integration strategy, and verification results.

> [!IMPORTANT]
> **Project status: early foundation / active development.** The current codebase provides a Rust workspace, a CLI, a local HTTP API, deterministic project-specification validation, and shallow acquisition plus manifest inventory for public GitHub repositories. Semantic repository analysis, dependency graphs, Feature Capsule extraction, compatibility planning, autonomous orchestration, and target-project integration are planned work; they are not implemented yet.

---

## Why Software Composer?

Building a new application often means solving problems that other projects have already solved: authentication, permissions, file uploads, payments, notifications, search, background jobs, and many others.

Existing approaches leave a gap:

- **Code generation** can create new implementations, but may duplicate mature work that already exists.
- **Package managers** can reuse published packages, but do not by themselves identify and extract a capability buried inside a larger application.
- **Copying files** is fragile because a feature may depend on configuration, shared utilities, data models, middleware, runtime services, or transitive code.
- **Repository search** can find candidate code, but finding code is not the same as proving that it can be reused safely.

Software Composer is intended to connect discovery to verified integration. It will reason about the capability as a unit, not just copy files that look relevant.

## What it is designed to do

The planned composition workflow is:

1. **Understand the project request.** Turn a project brief into a structured specification containing requested capabilities, priorities, and preferences.
2. **Discover candidate implementations.** Find repositories and packages that may already contain the requested behavior.
3. **Acquire reproducible source snapshots.** Pin each source to an immutable commit and record its Git tree identity.
4. **Understand repository structure.** Inspect languages, build systems, modules, symbols, imports, tests, configuration, and runtime relationships.
5. **Identify capability boundaries.** Locate the code and interfaces that implement a requested feature.
6. **Resolve dependency closure.** Determine what the capability needs to function, including transitive dependencies and external services. Uncertain relationships should remain explicitly unresolved rather than being guessed.
7. **Create a Feature Capsule.** Package the selected source, dependency information, interface and runtime requirements, applicable license notices, provenance, and verification evidence.
8. **Plan integration.** Compare the capsule with the target project's architecture and choose an appropriate reuse strategy, including adapters where required.
9. **Integrate and verify.** Apply changes in an isolated workspace, run relevant checks, and repair or reject integrations that do not meet their requirements.
10. **Deliver with provenance.** Explain what was reused, where it came from, what changed, what dependencies remain, and which checks passed or failed.

This is the intended end state—not a claim that every stage is available in the current version.

## What works today

| Capability | Current status |
| --- | --- |
| Rust workspace and shared domain model | Implemented |
| JSON project specification | Implemented |
| Deterministic specification validation | Implemented |
| CLI validation command | Implemented |
| Local Axum HTTP API | Implemented |
| Public GitHub HTTPS repository acquisition | Initial implementation |
| Commit SHA and Git tree SHA recording | Implemented |
| Build-manifest inventory and approximate language detection | Initial implementation |
| AST and semantic repository analysis | Planned |
| Dependency and symbol graph | Planned |
| Capability boundary detection | Planned |
| Feature Capsule extraction | Planned |
| Compatibility and integration planning | Planned |
| Sandboxed integration and verification | Planned |
| Autonomous agent orchestration | Planned |
| Provenance-backed composition report | Planned |

There is currently **no model provider or autonomous agent workflow wired into the application**. Project specifications are validated deterministically; validation does not start a composition run.

---

## Technology stack

The project is Rust-first. Rust is the primary language for the backend, domain model, CLI, repository tooling, and the future composition and verification infrastructure.

| Technology | Role |
| --- | --- |
| **Rust (edition 2024)** | Primary implementation language |
| **Cargo workspace** | Multi-crate build and dependency management |
| **Tokio** | Async runtime |
| **Axum** | HTTP API and routing |
| **Clap** | Command-line argument parsing |
| **Serde + Serde JSON** | Domain serialization and JSON specifications |
| **thiserror** | Explicit domain and subsystem errors |
| **tracing + tracing-subscriber** | Structured application logging |
| **url** | GitHub repository URL parsing and validation |
| **uuid** | Unique repository checkout directory names |
| **Git** | Shallow source acquisition and Git metadata inspection |

### Language strategy

- **Rust** is the core backend and orchestration language.
- **TypeScript** is the intended language for a future web frontend.
- Language-specific compiler, parser, and language-server tooling may be used when a repository analyzer needs it.
- **Python** may be used for optional adapters where its ecosystem provides a material advantage, but it is not a core runtime requirement.

The current repository does not yet contain the planned TypeScript frontend, language-specific AST analyzers, or model-provider integrations. Those should not be mistaken for existing dependencies.

## Architecture

The long-term architecture separates responsibilities into five planes:

| Plane | Responsibility |
| --- | --- |
| **Control plane** | CLI, API, orchestration, policies, workflow state, and metadata |
| **Intelligence plane** | Requirement decomposition, repository analysis, capability matching, dependency reasoning, and repair planning |
| **Composition plane** | Source acquisition, capability extraction, adapters, integration, and artifact assembly |
| **Execution plane** | Isolated builds, tests, static analysis, and runtime verification |
| **Evidence plane** | Source snapshots, dependency and license evidence, transformations, test results, and provenance |

The control plane must not execute untrusted repository code. Future execution workers will receive bounded jobs and return explicit artifacts and results.

### Reuse strategy

Composer is not intended to maximize the amount of copied code. It should choose the least invasive reuse strategy that satisfies the requirement:

1. **Published package** — depend on an existing package when it is suitable.
2. **Self-contained module** — reuse a module with a clear interface and manageable dependencies.
3. **Service or process boundary** — isolate a tightly coupled capability when direct reuse would create excessive coupling.
4. **Code transplantation** — move source only when its dependency closure and licensing obligations are understood and it can be verified.
5. **First-party implementation** — implement the missing behavior when reuse is unsafe, incompatible, or more expensive than replacement.

### Feature Capsules

A Feature Capsule is the planned, versioned unit of reusable functionality.

A capsule is intended to carry:

- The capability manifest and version.
- Selected source files and their original repository paths.
- Source repository URL, immutable revision, and snapshot identity.
- Dependency closure and dependency classifications.
- Runtime, configuration, and external-service requirements.
- Public interfaces and integration contracts.
- Applicable license files and notices.
- Any adapters or transformations applied.
- Verification commands, results, and evidence.

Dependencies should be classified as **required**, **optional**, **replaceable**, **external**, or **unresolved**. A capsule should preserve unknowns instead of presenting an incomplete dependency analysis as certainty.

---

## Repository structure

The current workspace is organized as follows:

~~~text
composer/
├── apps/
│   ├── api/                 # Axum HTTP API
│   └── cli/                 # composer command-line application
├── crates/
│   ├── core/                # Shared project specification and validation
│   └── repository/          # GitHub acquisition and manifest inventory
├── docs/
│   ├── adr/                 # Architecture Decision Records
│   └── architecture/        # Architecture, MVP plan, and threat model
├── examples/
│   └── project-spec.json    # Example project request
├── Cargo.toml               # Workspace configuration
├── Cargo.lock
└── rust-toolchain.toml
~~~

### Workspace crates

- **composer-core** — shared domain types, project requirements, priorities, constraints, and deterministic validation.
- **composer-repository** — GitHub URL validation, shallow repository acquisition, Git metadata collection, manifest inventory, and approximate language detection.
- **composer-api** — HTTP transport and API request/response handling.
- **composer-cli** — local developer interface for validating specifications, inspecting repositories, and starting the API.

New crates should be introduced when a subsystem has a stable contract and a meaningful independent test boundary.

---

## Getting started

### Requirements

- Rust stable with Rust **1.85 or newer** (edition 2024).
- Cargo, included with the Rust toolchain.
- Git 2.x available on your PATH.

Install Rust from [rustup.rs](https://rustup.rs/) and Git from [git-scm.com](https://git-scm.com/).

### Clone and build

~~~bash
git clone https://github.com/KUNDURUHIMANEESHREDDY/composer.git
cd composer

cargo build --workspace
~~~

### Validate a project specification

~~~bash
cargo run -p composer-cli -- validate-spec examples/project-spec.json
~~~

The command parses the JSON, validates its domain constraints, and prints the normalized specification.

### Inspect a public GitHub repository

~~~bash
cargo run -p composer-cli -- inspect https://github.com/serde-rs/serde
~~~

By default, the checkout is placed under **.composer/sources**. A custom workspace can be supplied:

~~~bash
cargo run -p composer-cli -- inspect https://github.com/serde-rs/serde --workspace ./repository-snapshots
~~~

The command prints a JSON inspection record containing the repository identity, checkout path, resolved commit SHA, Git tree SHA, discovered manifest paths, detected languages, and whether the inventory was truncated.

### Start the API

~~~bash
cargo run -p composer-api
~~~

The API listens on **127.0.0.1:3000** by default. Set **COMPOSER_BIND** to change the address:

~~~bash
COMPOSER_BIND=127.0.0.1:4000 cargo run -p composer-api
~~~

The CLI also exposes a server command:

~~~bash
cargo run -p composer-cli -- serve --bind 127.0.0.1:3000
~~~

---

## Project specifications

Composer accepts a structured JSON description of the project it is expected to assemble. The current ProjectSpec model contains a project name, description, one or more capability requirements, and optional constraints.

Example: [examples/project-spec.json](examples/project-spec.json)

~~~json
{
  "name": "Task Platform",
  "description": "A project management application assembled from reusable capabilities.",
  "capabilities": [
    {
      "id": "authentication",
      "description": "User sign-in, sessions, and authorization.",
      "priority": "critical"
    },
    {
      "id": "kanban-board",
      "description": "Boards, columns, cards, and drag-and-drop task movement.",
      "priority": "high"
    }
  ],
  "constraints": {
    "preferred_languages": ["rust", "typescript"]
  }
}
~~~

### Specification rules

- **name** is required, must not be empty, and is limited to 80 characters.
- **description** is required, must not be empty, and is limited to 4,000 characters.
- At least one capability is required.
- Capability IDs must be unique lowercase kebab-case identifiers.
- Each capability needs a non-empty description.
- Capability priority may be **critical**, **high**, **medium**, or **low**; it defaults to **medium**.
- Preferred language entries cannot be empty. They are preferences, not hard restrictions.
- Unknown JSON fields are rejected so that specification errors are not silently ignored.

---

## CLI reference

The current CLI binary is named **composer**.

| Command | Description |
| --- | --- |
| **composer validate-spec <PATH>** | Parse, validate, and print a project specification |
| **composer inspect <GITHUB_URL>** | Acquire and inspect a public GitHub repository |
| **composer serve [--bind <ADDRESS>]** | Start the local HTTP API |

Run the help command for the available arguments:

~~~bash
cargo run -p composer-cli -- --help
cargo run -p composer-cli -- inspect --help
~~~

## HTTP API

The current API is intentionally small and stateless.

| Method | Endpoint | Purpose |
| --- | --- | --- |
| **GET** | **/health** | Return service health and version |
| **POST** | **/v1/specifications/validate** | Validate and normalize a project specification |

### Health check

~~~bash
curl http://127.0.0.1:3000/health
~~~

### Validate a specification

~~~bash
curl -X POST http://127.0.0.1:3000/v1/specifications/validate \
  -H "Content-Type: application/json" \
  --data @examples/project-spec.json
~~~

A successful request returns a response in this shape:

~~~json
{
  "valid": true,
  "spec": {
    "name": "Task Platform",
    "description": "A project management application assembled from reusable capabilities.",
    "capabilities": [],
    "constraints": {}
  }
}
~~~

The example above illustrates the response envelope; the returned **spec** contains the submitted, validated values.

Error responses use:

- **400 Bad Request** — malformed JSON or an invalid request body.
- **422 Unprocessable Entity** — valid JSON that fails project-specification validation.

Validation does not create a project, persist state, search repositories, or start an agent workflow.

---

## Repository acquisition: current behavior and limits

The current **inspect** command is the first implementation slice of repository intelligence.

It currently:

- Accepts canonical HTTPS URLs in the form **https://github.com/owner/repository**.
- Rejects non-GitHub hosts, non-HTTPS URLs, credentials in the URL, custom ports, query strings, fragments, and nested repository paths.
- Clones the default branch as a shallow snapshot (**--depth 1**).
- Disables tags and recursive submodule fetching.
- Invokes Git through process argument arrays rather than a shell.
- Disables terminal prompts and restricts Git transport to HTTPS.
- Records the resolved commit SHA and Git tree SHA.
- Inventories recognized build manifests and derives an approximate language list from those manifests.

Recognized manifest names currently include:

- Rust: **Cargo.toml**
- JavaScript / TypeScript: **package.json**, **tsconfig.json**
- Python: **pyproject.toml**, **requirements.txt**
- Go: **go.mod**
- Java: **pom.xml**, **build.gradle**, **build.gradle.kts**
- PHP: **composer.json**
- Ruby: **Gemfile**
- Elixir: **mix.exs**
- C#: **\*.csproj**

Inventory traversal is bounded to a maximum depth of 6 and 20,000 visited files. Common generated or dependency directories such as **.git**, **node_modules**, **vendor**, **target**, **dist**, **.next**, **.venv**, and **__pycache__** are skipped. Symbolic links are not traversed. The inspection result includes an **inventory_truncated** flag.

### Important acquisition limitations

- Public GitHub HTTPS repositories only; private repositories are not supported yet.
- Only a shallow snapshot of the default branch is acquired; selecting arbitrary commits or branches is not implemented.
- Submodules are not fetched recursively.
- Language detection is manifest-based and approximate.
- This is inventory, not semantic analysis: no AST, symbol, import, or dependency graph is produced yet.
- Configurable clone timeouts, cancellation, repository-size checks, and disk budgets are still outstanding.
- **Repository code is not executed.** The acquisition command does not run build scripts, package lifecycle hooks, or tests.

---

## Security and trust model

External repositories are untrusted input. Source files, README content, comments, build definitions, package metadata, generated code, and test output must be treated as data—not as instructions that can override system policies or developer intent.

Current acquisition safeguards include:

- Strict GitHub HTTPS URL validation.
- Git invoked without a shell.
- HTTPS-only Git transport and disabled terminal prompts.
- No recursive submodule initialization.
- No execution of acquired repository code during cloning or inventory.
- Bounded manifest traversal and explicit symlink skipping.

These controls reduce risk during acquisition; they do **not** make arbitrary repository code safe to build or run.

The planned execution architecture calls for isolated, ephemeral workers with resource limits, restricted network access, no access to control-plane credentials, and explicit artifact boundaries. Stronger isolation may be needed for hostile code. Those execution controls are design requirements for future work, not capabilities currently delivered by this repository.

---

## Development

Format the workspace:

~~~bash
cargo fmt --all
~~~

Check formatting:

~~~bash
cargo fmt --all -- --check
~~~

Run Clippy:

~~~bash
cargo clippy --workspace --all-targets -- -D warnings
~~~

Run tests:

~~~bash
cargo test --workspace
~~~

Build all workspace packages:

~~~bash
cargo build --workspace
~~~

These are the project's development checks. Their inclusion here documents how to run them; it is not a claim that every check has been run on every revision.

---

## Roadmap

The MVP is focused on proving that a capability can be discovered, analyzed, extracted with its required dependencies, integrated into another project, and verified with inspectable evidence.

1. **Repository intelligence** — source inventory, language-aware parsing, modules, symbols, imports, tests, configuration, and license discovery.
2. **Dependency graph** — typed relationships with evidence, confidence, and explicit unknowns.
3. **Capability extraction** — feature boundaries, dependency closure, and versioned Feature Capsules.
4. **Compatibility planning** — target architecture inspection, equivalence detection, adapter contracts, and reuse strategy selection.
5. **Integration and verification** — disposable target workspaces, isolated execution, regression checks, and repair loops.
6. **Evidence-backed delivery** — provenance, source attribution, transformations, dependency decisions, and verification reports.
7. **Orchestration and interface** — durable workflow state, progress reporting, and a developer-facing experience.

The first end-to-end demonstration is intended to compose a small application from a small number of public repositories and a target template, with the source revisions, license evidence, extracted files, dependency closure, integration diff, and test results available for review.

---

## Design decisions and project documentation

The architecture and implementation decisions are recorded in the repository:

- [Architecture overview](docs/architecture/overview.md)
- [MVP plan](docs/architecture/mvp-plan.md)
- [Initial threat model](docs/architecture/threat-model.md)
- [ADR-0001: Rust-first core](docs/adr/0001-rust-first.md)
- [ADR-0002: Feature Capsules as the reuse artifact](docs/adr/0002-feature-capsules.md)
- [ADR-0003: GitHub HTTPS source acquisition](docs/adr/0003-github-source-acquisition.md)

These documents describe both current implementation boundaries and the intended system design. When they discuss future capabilities, those capabilities should be read as design direction rather than implemented behavior.

---

## Project status

Software Composer is an actively evolving engineering project. The current foundation is deliberately small: establish the domain model, define deterministic validation, acquire source repositories reproducibly, and keep untrusted code out of the control plane before building the deeper intelligence and execution layers.

The near-term priority is not to claim that an autonomous composer already works end to end. It is to build the repository intelligence, dependency reasoning, and verification foundations that would make autonomous software reuse dependable.

## License

No repository license has been declared yet. Until a license is added, do not assume that the source code is available for unrestricted reuse.
