//! Reuse an independent GitHub Rust library through Cargo's native pinned
//! Git dependency resolver. No algorithm rewriting, no host-side build.
//! Cargo caches must be provisioned beforehand using cargo fetch; the source
//! is executed only by Composer's existing isolated offline Docker verifier.
use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    time::Instant,
};

use composer_repository::GitHubRepository;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    VerifiedRustError,
    reviewed::validate_contract_test,
    verified::{
        copy_bounded_target_with_workspace_dirs, docker_build_with_git_cache,
        inspect_local_image_id, nonroot_container_user,
    },
};

const GIT_EVIDENCE: &str = "composer-upstream-git-evidence.json";

#[derive(Debug, Clone)]
pub struct UpstreamGitOptions {
    pub target_root: PathBuf,
    pub output_root: PathBuf,
    /// Clone of an independently reviewed GitHub Rust repository.
    pub upstream_checkout: PathBuf,
    pub upstream_repository: String,
    /// Exact, independently reviewed 40-hex Git commit.
    pub upstream_commit: String,
    /// Package name declared in the upstream Git Cargo.toml.
    pub package: String,
    /// Existing upstream public API path relative to crate root.
    pub public_symbol: String,
    /// Target-facing name used in target-owned contract tests.
    pub target_alias: String,
    /// Optional independently reviewed target API signature; typed glue only.
    pub target_signature: Option<String>,
    /// Infer only a top-level public fn(&str[, &str...]) -> String from the pinned checkout.
    pub infer_owned_string_signature: bool,
    pub contract_test: PathBuf,
    pub registry_cache: PathBuf,
    /// Cargo's pre-fetched git/ directory only (never entire Cargo home).
    pub git_cache: PathBuf,
    pub license_reviewed: bool,
    /// Optional trusted OpenPGP signer fingerprint (40 or 64 hex).
    /// Requiring it rejects unsigned or untrusted upstream commits.
    pub required_signer_fingerprint: Option<String>,
    /// Explicitly test all library and integration tests, excluding third-party
    /// benchmarks/examples/binaries. Never automatically lower verification.
    pub contract_focused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamGitResult {
    pub output_root: PathBuf,
    pub package: String,
    pub public_symbol: String,
    pub target_alias: String,
    pub target_signature: Option<String>,
    pub inferred_owned_string_signature: bool,
    pub upstream_repository: String,
    pub upstream_commit: String,
    pub lockfile_git_commit_verified: bool,
    pub source_checkout_commit_verified: bool,
    pub source_license_sha256: String,
    /// Immutable Git tree object of the independently reviewed upstream commit.
    pub reviewed_source_tree_oid: String,
    /// Blob object ID independently checked against the clean checkout.
    pub reviewed_manifest_blob_oid: String,
    pub reviewed_license_blob_oid: String,
    pub source_git_objects_verified: bool,
    /// True only when an explicitly required signature was verified by Git.
    pub source_commit_signature_verified: bool,
    pub verified_signer_fingerprint: Option<String>,
    pub original_target_unchanged: bool,
    pub offline_tests_passed: bool,
    /// Explicit verification scope; not equivalent to passing --all-targets.
    pub verification_scope: String,
    pub evidence_path: PathBuf,
    pub elapsed_seconds: f64,
}

/// Exact workspace member directories are copied through the existing
/// bounded, symlink-safe snapshot. Cargo glob patterns and traversal refuse
/// integration rather than silently omitting members.
fn workspace_member_roots(cargo: &toml::Value) -> Result<Vec<String>, VerifiedRustError> {
    let Some(workspace) = cargo.get("workspace") else {
        return Ok(Vec::new());
    };
    let members = workspace.get("members").and_then(toml::Value::as_array);
    let mut roots = std::collections::BTreeSet::new();
    if let Some(members) = members {
        if members.len() > 32 {
            return Err(VerifiedRustError::Dependency(
                "too many Cargo workspace members".into(),
            ));
        }
        for member in members {
            let path = member.as_str().ok_or_else(|| {
                VerifiedRustError::Unsafe("workspace member must be a plain directory path".into())
            })?;
            if path == "." {
                continue;
            }
            let parts = path.split('/').collect::<Vec<_>>();
            if parts.is_empty()
                || parts.iter().any(|part| {
                    part.is_empty()
                        || part.starts_with('.')
                        || part
                            .bytes()
                            .any(|c| !c.is_ascii_alphanumeric() && c != b'-' && c != b'_')
                })
            {
                return Err(VerifiedRustError::Unsafe(
                    "workspace member traversal, glob, or nonstandard path refused".into(),
                ));
            }
            roots.insert(parts[0].to_owned());
        }
    }
    Ok(roots.into_iter().collect())
}

#[test]
fn workspace_member_paths_are_bounded_and_nontraversing() {
    let valid: toml::Value = toml::from_str("[package]\nname='x'\nversion='0.1.0'\n[workspace]\nmembers=['.', 'crates/one', 'tests/other']").unwrap();
    assert_eq!(
        workspace_member_roots(&valid).unwrap(),
        vec!["crates".to_owned(), "tests".to_owned()]
    );
    for bad in ["../escape", "crates/*", "/absolute", "crates/../escape"] {
        let config: toml::Value =
            toml::from_str(&format!("[workspace]\nmembers=['{bad}']")).unwrap();
        assert!(workspace_member_roots(&config).is_err(), "{bad}");
    }
}

fn ascii_ident(input: &str) -> bool {
    !input.is_empty()
        && input.len() <= 80
        && input != "_"
        && input
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && input
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn valid_package(input: &str) -> bool {
    !input.is_empty()
        && input.len() <= 64
        && input
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && input
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_'))
}

fn valid_public_path(input: &str) -> bool {
    let names = input.split("::").collect::<Vec<_>>();
    !names.is_empty() && names.len() <= 4 && names.iter().all(|part| ascii_ident(part))
}

fn git_read(path: &Path, args: &[&str]) -> Result<String, VerifiedRustError> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(path)
        // `git status` consults checkout-local .git/config before the source is
        // trusted. `core.fsmonitor` can execute an arbitrary helper on the HOST.
        // Disable it (and optional index caching) with highest-precedence CLI
        // config. The existing Git executable does only read-only metadata.
        .args([
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
        ])
        .args(["-c", "gpg.program=gpg"])
        .args(args)
        // Third-party Git metadata must not inherit operator-global filter
        // commands or environment-injected Git configuration.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_EXTERNAL_DIFF");
    let out = command
        .output()
        .map_err(|err| VerifiedRustError::Unsafe(format!("Git metadata unavailable: {err}")))?;
    if !out.status.success() || out.stdout.len() > 65536 {
        return Err(VerifiedRustError::Unsafe(
            "Git checkout metadata could not be verified".into(),
        ));
    }
    String::from_utf8(out.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| VerifiedRustError::Unsafe("non-UTF8 Git metadata".into()))
}

/// Git status can run executable clean filters from checkout-local Git
/// configuration. Git's own config reader is safe for discovery; refuse any
/// executable filter before Git considers the untrusted worktree's attributes.
fn reject_host_git_filter_commands(checkout: &Path) -> Result<(), VerifiedRustError> {
    let keys = git_read(checkout, &["config", "--includes", "--list", "--name-only"])?;
    for key in keys.lines() {
        let key = key.trim().to_ascii_lowercase();
        if key.starts_with("filter.")
            && (key.ends_with(".clean") || key.ends_with(".smudge") || key.ends_with(".process"))
        {
            return Err(VerifiedRustError::Unsafe(
                "checkout Git filter helper configuration cannot execute on the host".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ReviewedSourceEvidence {
    license_bytes: Vec<u8>,
    license_sha256: String,
    tree_oid: String,
    manifest_blob_oid: String,
    license_blob_oid: String,
    license_path: String,
}

fn tracked_blob_matches(
    checkout: &Path,
    revision: &str,
    relative_path: &str,
) -> Result<String, VerifiedRustError> {
    // Do not accept a path whose checkout bytes differ from the Git tree.
    // Git (not custom hash logic) computes and verifies the stored blob ID.
    let git_path = format!("{revision}:{relative_path}");
    let expected = git_read(checkout, &["rev-parse", "--verify", &git_path])?;
    let actual = git_read(
        checkout,
        &["hash-object", "--no-filters", "--", relative_path],
    )?;
    if expected.len() != 40
        || expected != actual
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(VerifiedRustError::Unsafe(format!(
            "reviewed source file {relative_path} differs from the pinned Git object"
        )));
    }
    Ok(expected)
}

fn verify_trusted_upstream_signer(
    checkout: &Path,
    expected: &str,
) -> Result<String, VerifiedRustError> {
    // Reuse installed Git and the user's trusted OpenPGP keyring; no custom
    // signature verifier. Never accept unsigned or unknown-trust signatures.
    if !matches!(expected.len(), 40 | 64) || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(VerifiedRustError::Unsafe(
            "required upstream signer fingerprint must be 40 or 64 hex characters".into(),
        ));
    }
    git_read(checkout, &["verify-commit", "HEAD"]).map_err(|_| {
        VerifiedRustError::Unsafe(
            "upstream commit signature verification failed or signing key unavailable".into(),
        )
    })?;
    let trust = git_read(checkout, &["log", "-1", "--format=%G?", "HEAD"])?;
    let actual = git_read(checkout, &["log", "-1", "--format=%GF", "HEAD"])?;
    if trust != "G" || !actual.eq_ignore_ascii_case(expected) {
        return Err(VerifiedRustError::Unsafe(
            "upstream signing fingerprint does not match a trusted valid Git signature".into(),
        ));
    }
    Ok(actual.to_ascii_uppercase())
}

fn checkout_license(
    checkout: &Path,
    package: &str,
    repo: &GitHubRepository,
    rev: &str,
) -> Result<ReviewedSourceEvidence, VerifiedRustError> {
    let meta = fs::symlink_metadata(checkout)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe(
            "source checkout must be a regular directory".into(),
        ));
    }
    if git_read(checkout, &["rev-parse", "--verify", "HEAD^{commit}"])? != rev {
        return Err(VerifiedRustError::Unsafe(
            "reviewed checkout HEAD differs from requested Git SHA".into(),
        ));
    }
    // Scan effective repository-local + included config before Git status.
    // System/global configuration is already disabled in git_read().
    reject_host_git_filter_commands(checkout)?;
    if !git_read(
        checkout,
        &["status", "--porcelain", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Err(VerifiedRustError::Unsafe(
            "reviewed checkout has uncommitted or untracked files".into(),
        ));
    }
    let origin = git_read(checkout, &["remote", "get-url", "origin"])?;
    let observed = GitHubRepository::parse(&origin).map_err(|_| {
        VerifiedRustError::Unsafe("source checkout origin is not canonical HTTPS GitHub".into())
    })?;
    if observed.clone_url != repo.clone_url {
        return Err(VerifiedRustError::Unsafe(
            "source checkout origin differs from requested source".into(),
        ));
    }
    let cargo_path = checkout.join("Cargo.toml");
    let cargo_meta = fs::symlink_metadata(&cargo_path)?;
    if !cargo_meta.is_file()
        || cargo_meta.file_type().is_symlink()
        || cargo_meta.len() > 1024 * 1024
    {
        return Err(VerifiedRustError::Unsafe(
            "source Cargo.toml is unsafe or oversized".into(),
        ));
    }
    let cargo: toml::Value = toml::from_str(&fs::read_to_string(&cargo_path)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    if cargo
        .get("package")
        .and_then(|table| table.get("name"))
        .and_then(toml::Value::as_str)
        != Some(package)
    {
        return Err(VerifiedRustError::Unsafe(
            "source checkout package name does not match reviewed source".into(),
        ));
    }
    // Check the actual on-disk manifest against the immutable Git object,
    // not only the ref name and worktree "clean" status.
    let tree_oid = git_read(checkout, &["rev-parse", "--verify", "HEAD^{tree}"])?;
    if tree_oid.len() != 40 || !tree_oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(VerifiedRustError::Unsafe(
            "invalid upstream Git tree object".into(),
        ));
    }
    let manifest_blob_oid = tracked_blob_matches(checkout, rev, "Cargo.toml")?;
    // Unpublished Git-only libraries often supply a LICENSE file without
    // declaring the license in Cargo.toml. Keep the explicit review gate and
    // hash/copy the committed LICENSE file rather than inventing a declaration.
    for name in [
        "LICENSE",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "LICENSE.md",
        "COPYING",
    ] {
        let file = checkout.join(name);
        let Ok(meta) = fs::symlink_metadata(&file) else {
            continue;
        };
        if !meta.is_file()
            || meta.file_type().is_symlink()
            || meta.len() == 0
            || meta.len() > 512 * 1024
        {
            return Err(VerifiedRustError::Unsafe(
                "upstream license file is unsafe".into(),
            ));
        }
        let bytes = fs::read(&file)?;
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        let license_blob_oid = tracked_blob_matches(checkout, rev, name)?;
        return Ok(ReviewedSourceEvidence {
            license_bytes: bytes,
            license_sha256: checksum,
            tree_oid,
            manifest_blob_oid,
            license_blob_oid,
            license_path: name.to_owned(),
        });
    }
    Err(VerifiedRustError::Unsafe(
        "source checkout has no regular license evidence file".into(),
    ))
}

fn validate_git_lock(
    lockfile: &Path,
    package: &str,
    repo: &GitHubRepository,
    rev: &str,
) -> Result<String, VerifiedRustError> {
    let metadata = fs::symlink_metadata(lockfile)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 8 * 1024 * 1024
    {
        return Err(VerifiedRustError::Dependency(
            "Git dependency lockfile invalid".into(),
        ));
    }
    let cargo: toml::Value = toml::from_str(&fs::read_to_string(lockfile)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    let entries = cargo
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| VerifiedRustError::Dependency("Cargo.lock has no packages".into()))?;
    let bare = repo.clone_url.trim_end_matches(".git");
    let candidates = entries
        .iter()
        .filter_map(|entry| {
            if entry.get("name").and_then(toml::Value::as_str) != Some(package) {
                return None;
            }
            let source = entry.get("source").and_then(toml::Value::as_str)?;
            let accepted = [
                format!("git+{}?rev={rev}#{rev}", repo.clone_url),
                format!("git+{bare}?rev={rev}#{rev}"),
            ];
            if accepted.iter().any(|possible| possible == source) {
                Some(source)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err(VerifiedRustError::Dependency(
            "Cargo.lock did not uniquely resolve requested exact Git revision".into(),
        ));
    }
    Ok(candidates[0].to_owned())
}

/// Publish a new target copy ONLY if a reviewed Git checkout and the Cargo
/// lockfile point to the exact same commit, and target-owned tests pass in an
/// offline, resource-bounded Docker container.
pub fn verify_upstream_git(
    options: &UpstreamGitOptions,
) -> Result<UpstreamGitResult, VerifiedRustError> {
    if !options.license_reviewed {
        return Err(VerifiedRustError::ReviewRequired);
    }
    let repo = GitHubRepository::parse(&options.upstream_repository).map_err(|_| {
        VerifiedRustError::Unsafe("canonical public HTTPS GitHub URL required".into())
    })?;
    let rev = &options.upstream_commit;
    if rev.len() != 40
        || !rev.bytes().all(|c| c.is_ascii_hexdigit())
        || !valid_package(&options.package)
        || !valid_public_path(&options.public_symbol)
        || !ascii_ident(&options.target_alias)
    {
        return Err(VerifiedRustError::Unsafe(
            "Git SHA, package, public symbol or target alias invalid".into(),
        ));
    }
    if options.infer_owned_string_signature && options.target_signature.is_some() {
        return Err(VerifiedRustError::Unsafe(
            "specify either a reviewed target signature or --infer-owned-string-signature, not both".into(),
        ));
    }
    // Checkout is verified clean, at the exact requested commit and public
    // upstream origin BEFORE its source code can influence API inference.
    let reviewed = checkout_license(&options.upstream_checkout, &options.package, &repo, rev)?;
    let effective_signature = if options.infer_owned_string_signature {
        let lib = options.upstream_checkout.join("src/lib.rs");
        let meta = fs::symlink_metadata(&lib)?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 512 * 1024 {
            return Err(VerifiedRustError::Unsafe(
                "bounded API inference requires a regular pinned upstream src/lib.rs of at most 512 KiB".into(),
            ));
        }
        let upstream_source = fs::read_to_string(&lib)?;
        Some(crate::api_discovery::infer_owned_string_target_signature(
            &upstream_source,
            &options.public_symbol,
            &options.target_alias,
        ).ok_or_else(|| VerifiedRustError::Unsafe(
            "no unique supported public borrowed-string function in pinned upstream Git source".into(),
        ))?)
    } else {
        options.target_signature.clone()
    };
    let typed_bridge = crate::reviewed_upstream_adapter::reviewed_string_adapter(
        &format!("composer_git_{}", options.package.replace('-', "_")),
        &options.public_symbol,
        &options.target_alias,
        effective_signature.as_deref(),
    )?;
    let signer = options
        .required_signer_fingerprint
        .as_deref()
        .map(|fingerprint| verify_trusted_upstream_signer(&options.upstream_checkout, fingerprint))
        .transpose()?;
    let original = options.target_root.canonicalize()?;
    validate_contract_test(&original, &options.contract_test, &options.target_alias)?;
    let target_manifest = original.join("Cargo.toml");
    let mut cargo: toml::Value = toml::from_str(&fs::read_to_string(&target_manifest)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    if cargo.get("package").is_none()
        || cargo.get("patch").is_some()
        || cargo.get("replace").is_some()
    {
        return Err(VerifiedRustError::Dependency(
            "Git capability import requires a root library package without patch/replace overrides"
                .into(),
        ));
    }
    let workspace_dirs = workspace_member_roots(&cargo)?;
    let lib = original.join("src/lib.rs");
    let meta = fs::symlink_metadata(&lib)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe(
            "target library file is unsafe".into(),
        ));
    }
    let original_lib = fs::read_to_string(&lib)?;
    let dependency_key = format!("composer_git_{}", options.package.replace('-', "_"));
    let dependencies = cargo
        .as_table_mut()
        .ok_or_else(|| VerifiedRustError::Toml("target manifest is not a table".into()))?
        .entry("dependencies".to_owned())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| {
            VerifiedRustError::Dependency("target dependencies are not a table".into())
        })?;
    if dependencies.contains_key(&dependency_key)
        || original_lib.contains(&format!("pub use {dependency_key}::"))
    {
        return Err(VerifiedRustError::Unsafe(
            "Git dependency alias already exists in target".into(),
        ));
    }
    let mut dependency = toml::map::Map::new();
    dependency.insert(
        "package".into(),
        toml::Value::String(options.package.clone()),
    );
    dependency.insert("git".into(), toml::Value::String(repo.clone_url.clone()));
    dependency.insert("rev".into(), toml::Value::String(rev.clone()));
    dependencies.insert(dependency_key.clone(), toml::Value::Table(dependency));
    let destination = &options.output_root;
    if destination.exists()
        || !matches!(
            destination.components().next_back(),
            Some(Component::Normal(_))
        )
    {
        return Err(VerifiedRustError::OutputExists(destination.clone()));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| VerifiedRustError::Unsafe("Git output requires an existing parent".into()))?
        .canonicalize()?;
    let name = destination
        .file_name()
        .ok_or_else(|| VerifiedRustError::Unsafe("Git output requires a final name".into()))?;
    let output = parent.join(name);
    if output.starts_with(&original) || original.starts_with(&output) {
        return Err(VerifiedRustError::Unsafe(
            "output overlaps original target".into(),
        ));
    }
    let id = Uuid::new_v4();
    let staged = parent.join(format!(".composer-git-{id}.tmp"));
    let results = parent.join(format!(".composer-git-result-{id}.tmp"));
    fs::create_dir(&staged)?;
    if let Err(err) = fs::create_dir(&results) {
        let _ = fs::remove_dir_all(&staged);
        return Err(err.into());
    }
    let result = (|| {
        copy_bounded_target_with_workspace_dirs(&original, &staged, &workspace_dirs)?;
        fs::write(
            staged.join("Cargo.toml"),
            toml::to_string_pretty(&cargo)
                .map_err(|err| VerifiedRustError::Toml(err.to_string()))?,
        )?;
        let mut library = original_lib.clone();
        if let Some(adapter) = &typed_bridge {
            library.push_str(adapter);
        } else {
            library.push_str(&format!(
                "\n// Composer: reused pinned upstream Git capability, not regenerated\n\
                 pub use {dependency_key}::{} as {};\n",
                options.public_symbol, options.target_alias,
            ));
        }
        fs::write(staged.join("src/lib.rs"), library)?;
        let image_id = inspect_local_image_id()?;
        let container_user = nonroot_container_user()?;
        let start = Instant::now();
        let logs = docker_build_with_git_cache(
            &staged,
            &results,
            Some(&options.registry_cache),
            &options.git_cache,
            &image_id,
            &container_user,
            options.contract_focused,
        )?;
        let elapsed = start.elapsed().as_secs_f64();
        let source = validate_git_lock(&results.join("Cargo.lock"), &options.package, &repo, rev)?;
        fs::copy(results.join("Cargo.lock"), staged.join("Cargo.lock"))?;
        let licenses = staged.join("third_party_licenses");
        fs::create_dir_all(&licenses)?;
        let notice = format!("{}-git-license.txt", options.package);
        fs::write(licenses.join(&notice), &reviewed.license_bytes)?;
        let evidence = serde_json::json!({
            "schema_version": 1,
            "mode": "reuse_pinned_git_dependency",
            "source_repo": repo.clone_url,
            "source_commit": rev,
            "lockfile_source": source,
            "source_checkout_commit_verified": true,
            "cargo_lock_commit_verified": true,
            "license_sha256": reviewed.license_sha256,
            "license_notice": format!("third_party_licenses/{notice}"),
            "reviewed_license_source_path": reviewed.license_path,
            "reviewed_source_tree_oid": reviewed.tree_oid,
            "reviewed_manifest_blob_oid": reviewed.manifest_blob_oid,
            "reviewed_license_blob_oid": reviewed.license_blob_oid,
            "source_git_objects_verified": true,
            "source_commit_signature_verified": signer.is_some(),
            "verified_signer_fingerprint": signer,
            "review_attested": true,
            "target_contract": options.contract_test,
            "public_symbol": options.public_symbol,
            "target_alias": options.target_alias,
            "target_signature": effective_signature,
            "signature_inferred_from_pinned_source": options.infer_owned_string_signature,
            "adapter_kind": if typed_bridge.is_some() { "borrow_owned_strings_as_str" } else { "direct_reexport" },
            "offline_tests_passed": true,
            "network_disabled": true,
            "docker_memory_limit_mb": crate::verified::selected_verifier_memory_mb()?,
            "docker_cpu_limit": crate::verified::selected_verifier_cpus()?,
            "docker_all_features": crate::verified::selected_verifier_features()?,
            "cargo_workspace_root_package": cargo.get("workspace").is_some(),
            "verification_scope": if options.contract_focused { "library_and_integration_tests" } else { "all_targets" },
            "third_party_benchmarks_verified": !options.contract_focused,
            "docker_image_id": image_id,
            "docker_image_reference": crate::verified::selected_verifier_image()?,
            "original_target_unchanged": true,
            "elapsed_seconds": elapsed,
            "log_tail": logs,
            "limitations": "Reviewed Git commit pinned by Cargo and checked against original clean checkout and Cargo.lock; independent target tests are limited and Docker is not VM-grade isolation."
        });
        fs::write(
            staged.join(GIT_EVIDENCE),
            serde_json::to_vec_pretty(&evidence)?,
        )?;
        fs::rename(&staged, &output)?;
        Ok(UpstreamGitResult {
            output_root: output.clone(),
            package: options.package.clone(),
            public_symbol: options.public_symbol.clone(),
            target_alias: options.target_alias.clone(),
            target_signature: effective_signature.clone(),
            inferred_owned_string_signature: options.infer_owned_string_signature,
            upstream_repository: repo.clone_url.clone(),
            upstream_commit: rev.clone(),
            lockfile_git_commit_verified: true,
            source_checkout_commit_verified: true,
            source_license_sha256: reviewed.license_sha256.clone(),
            reviewed_source_tree_oid: reviewed.tree_oid.clone(),
            reviewed_manifest_blob_oid: reviewed.manifest_blob_oid.clone(),
            reviewed_license_blob_oid: reviewed.license_blob_oid.clone(),
            source_git_objects_verified: true,
            source_commit_signature_verified: signer.is_some(),
            verified_signer_fingerprint: signer.clone(),
            original_target_unchanged: true,
            offline_tests_passed: true,
            verification_scope: if options.contract_focused {
                "library_and_integration_tests".into()
            } else {
                "all_targets".into()
            },
            evidence_path: output.join(GIT_EVIDENCE),
            elapsed_seconds: elapsed,
        })
    })();
    let _ = fs::remove_dir_all(&results);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staged);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_pinned_git_source_inputs() {
        assert!(valid_public_path("str::jaro"));
        assert!(!valid_public_path("str::jaro!"));
        assert!(!valid_public_path("::jaro"));
        assert!(valid_package("strsim"));
        assert!(!valid_package("../strsim"));
        let opts = UpstreamGitOptions {
            target_root: PathBuf::from("unknown"),
            output_root: PathBuf::from("unused"),
            upstream_checkout: PathBuf::from("untrusted"),
            upstream_repository: "https://github.com/rapidfuzz/strsim-rs".into(),
            upstream_commit: "main".into(),
            package: "strsim".into(),
            public_symbol: "jaro".into(),
            target_alias: "ported_jaro".into(),
            target_signature: None,
            infer_owned_string_signature: false,
            contract_test: PathBuf::from("tests/contract.rs"),
            registry_cache: PathBuf::from("missing-registry"),
            git_cache: PathBuf::from("missing-git"),
            license_reviewed: false,
            required_signer_fingerprint: None,
            contract_focused: false,
        };
        assert!(matches!(
            verify_upstream_git(&opts),
            Err(VerifiedRustError::ReviewRequired)
        ));
    }

    #[test]
    fn checkout_local_and_included_filter_helpers_are_refused_cross_platform() {
        let root =
            std::env::temp_dir().join(format!("composer-filter-crossplatform-{}", Uuid::new_v4()));
        let repo = root.join("checkout");
        fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git fixture: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        assert!(reject_host_git_filter_commands(&repo).is_ok());
        // A checkout's .git/config is not permitted to introduce executable
        // smudge/clean/process helpers into host-side metadata verification.
        git(&[
            "config",
            "filter.attack.process",
            "untrusted-helper-should-not-run",
        ]);
        assert!(reject_host_git_filter_commands(&repo).is_err());
        git(&["config", "--unset", "filter.attack.process"]);

        let included = root.join("included-config");
        fs::write(
            &included,
            "[filter \"attack\"]\nsmudge = untrusted-helper-should-not-run\n",
        )
        .unwrap();
        git(&["config", "include.path", included.to_str().unwrap()]);
        assert!(reject_host_git_filter_commands(&repo).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn host_git_metadata_disables_checkout_controlled_fsmonitor() {
        use std::os::unix::fs::PermissionsExt;

        let base = std::env::temp_dir().join(format!("composer-git-fsmonitor-{}", Uuid::new_v4()));
        let checkout = base.join("checkout");
        fs::create_dir_all(&checkout).unwrap();

        // Build a disposable legitimate Git checkout, then add a malicious
        // repository-local fsmonitor helper. A regular host `git status`
        // would execute this helper before Composer has reviewed the source.
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&checkout)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "disposable Git setup failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "composer-tests@example.invalid"]);
        git(&["config", "user.name", "Composer CI"]);
        fs::write(checkout.join("README.md"), "legitimate reviewed source\n").unwrap();
        git(&["add", "README.md"]);
        git(&["commit", "-qm", "create fixture"]);

        let marker = base.join("host-helper-executed");
        let helper = base.join("untrusted-monitor.sh");
        fs::write(
            &helper,
            format!(
                "#!/bin/sh\nprintf triggered > '{}'\necho /\n",
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        git(&["config", "core.fsmonitor", helper.to_str().unwrap()]);

        let clean = git_read(
            &checkout,
            &["status", "--porcelain", "--untracked-files=all"],
        )
        .expect("Composer must still verify a clean checkout");
        assert!(clean.is_empty());
        assert!(
            !marker.exists(),
            "host Git metadata executed repository-controlled fsmonitor code"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn checkout_filter_commands_are_refused_without_executing_host_helpers() {
        use std::os::unix::fs::PermissionsExt;

        let base = std::env::temp_dir().join(format!("composer-git-filter-{}", Uuid::new_v4()));
        let checkout = base.join("repo");
        fs::create_dir_all(&checkout).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&checkout)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git fixture error: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "Composer CI"]);
        git(&["config", "user.email", "composer-tests@example.invalid"]);
        fs::write(
            checkout.join(".gitattributes"),
            "payload.txt filter=attack\n",
        )
        .unwrap();
        fs::write(checkout.join("payload.txt"), "verified source\n").unwrap();
        git(&["add", ".gitattributes", "payload.txt"]);
        git(&["commit", "-qm", "original clean checkout"]);

        let marker = base.join("filter-executed-on-host");
        let helper = base.join("malicious-clean.sh");
        fs::write(
            &helper,
            format!("#!/bin/sh\nprintf ran > '{}'\ncat\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        git(&["config", "filter.attack.clean", helper.to_str().unwrap()]);
        assert!(reject_host_git_filter_commands(&checkout).is_err());
        assert!(
            !marker.exists(),
            "configured filter executed during config scan"
        );

        // Git config include.path can load another checkout-controlled filter.
        git(&["config", "--unset", "filter.attack.clean"]);
        let extra = base.join("included-config");
        fs::write(
            &extra,
            format!("[filter \"attack\"]\nclean = {}\n", helper.display()),
        )
        .unwrap();
        git(&["config", "include.path", extra.to_str().unwrap()]);
        assert!(reject_host_git_filter_commands(&checkout).is_err());
        assert!(
            !marker.exists(),
            "included Git filter executed during config scan"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn lockfile_requires_exact_git_commit_not_only_package_name() {
        let base = std::env::temp_dir().join(format!("composer-git-lock-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let lock = base.join("Cargo.lock");
        let repo = GitHubRepository::parse("https://github.com/rapidfuzz/strsim-rs").unwrap();
        let sha = "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26";
        fs::write(
            &lock,
            format!("version = 3\n[[package]]\nname = 'strsim'\nversion='0.11.1'\nsource='git+{}?rev={sha}#{sha}'\n", repo.clone_url),
        ).unwrap();
        assert!(validate_git_lock(&lock, "strsim", &repo, sha).is_ok());
        let wrong_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert!(validate_git_lock(&lock, "strsim", &repo, wrong_sha).is_err());
        fs::remove_dir_all(base).unwrap();
    }
}
