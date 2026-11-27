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
        copy_bounded_target, docker_build_with_git_cache, inspect_local_image_id,
        nonroot_container_user,
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
    pub contract_test: PathBuf,
    pub registry_cache: PathBuf,
    /// Cargo's pre-fetched git/ directory only (never entire Cargo home).
    pub git_cache: PathBuf,
    pub license_reviewed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamGitResult {
    pub output_root: PathBuf,
    pub package: String,
    pub public_symbol: String,
    pub target_alias: String,
    pub upstream_repository: String,
    pub upstream_commit: String,
    pub lockfile_git_commit_verified: bool,
    pub source_checkout_commit_verified: bool,
    pub source_license_sha256: String,
    pub original_target_unchanged: bool,
    pub offline_tests_passed: bool,
    pub evidence_path: PathBuf,
    pub elapsed_seconds: f64,
}

fn ascii_ident(input: &str) -> bool {
    !input.is_empty()
        && input.len() <= 80
        && input != "_"
        && input.bytes().next().is_some_and(|c| c.is_ascii_alphabetic())
        && input.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn valid_package(input: &str) -> bool {
    !input.is_empty()
        && input.len() <= 64
        && input.bytes().next().is_some_and(|c| c.is_ascii_alphabetic())
        && input.bytes().all(|c| c.is_ascii_lowercase()
            || c.is_ascii_digit() || matches!(c, b'-' | b'_'))
}

fn valid_public_path(input: &str) -> bool {
    let names = input.split("::").collect::<Vec<_>>();
    !names.is_empty() && names.len() <= 4
        && names.iter().all(|part| ascii_ident(part))
}

fn git_read(path: &Path, args: &[&str]) -> Result<String, VerifiedRustError> {
    let mut command = Command::new("git");
    command.arg("-C").arg(path)
        .args(["-c", "protocol.ext.allow=never"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "");
    let out = command.output().map_err(|err| {
        VerifiedRustError::Unsafe(format!("Git metadata unavailable: {err}"))
    })?;
    if !out.status.success() || out.stdout.len() > 65536 {
        return Err(VerifiedRustError::Unsafe(
            "Git checkout metadata could not be verified".into()
        ));
    }
    String::from_utf8(out.stdout).map(|value| value.trim().to_owned())
        .map_err(|_| VerifiedRustError::Unsafe("non-UTF8 Git metadata".into()))
}

fn checkout_license(
    checkout: &Path,
    package: &str,
    repo: &GitHubRepository,
    rev: &str,
) -> Result<(Vec<u8>, String), VerifiedRustError> {
    let meta = fs::symlink_metadata(checkout)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe("source checkout must be a regular directory".into()));
    }
    if git_read(checkout, &["rev-parse", "--verify", "HEAD^{commit}"])? != rev {
        return Err(VerifiedRustError::Unsafe("reviewed checkout HEAD differs from requested Git SHA".into()));
    }
    if !git_read(checkout, &["status", "--porcelain", "--untracked-files=all"])?.is_empty() {
        return Err(VerifiedRustError::Unsafe("reviewed checkout has uncommitted or untracked files".into()));
    }
    let origin = git_read(checkout, &["remote", "get-url", "origin"])?;
    let observed = GitHubRepository::parse(&origin).map_err(|_| {
        VerifiedRustError::Unsafe("source checkout origin is not canonical HTTPS GitHub".into())
    })?;
    if observed.clone_url != repo.clone_url {
        return Err(VerifiedRustError::Unsafe("source checkout origin differs from requested source".into()));
    }
    let cargo_path = checkout.join("Cargo.toml");
    let cargo_meta = fs::symlink_metadata(&cargo_path)?;
    if !cargo_meta.is_file() || cargo_meta.file_type().is_symlink() || cargo_meta.len() > 1024 * 1024 {
        return Err(VerifiedRustError::Unsafe("source Cargo.toml is unsafe or oversized".into()));
    }
    let cargo: toml::Value = toml::from_str(&fs::read_to_string(&cargo_path)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    if cargo.get("package")
        .and_then(|table| table.get("name"))
        .and_then(toml::Value::as_str) != Some(package)
        || cargo.get("package")
            .and_then(|table| table.get("license"))
            .and_then(toml::Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(VerifiedRustError::Unsafe(
            "source checkout package/license metadata not confirmed".into()
        ));
    }
    for name in ["LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "LICENSE.md", "COPYING"] {
        let file = checkout.join(name);
        let Ok(meta) = fs::symlink_metadata(&file) else { continue };
        if !meta.is_file() || meta.file_type().is_symlink()
            || meta.len() == 0 || meta.len() > 512 * 1024
        {
            return Err(VerifiedRustError::Unsafe("upstream license file is unsafe".into()));
        }
        let bytes = fs::read(&file)?;
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        return Ok((bytes, checksum));
    }
    Err(VerifiedRustError::Unsafe(
        "source checkout has no regular license evidence file".into()
    ))
}

fn validate_git_lock(
    lockfile: &Path,
    package: &str,
    repo: &GitHubRepository,
    rev: &str,
) -> Result<String, VerifiedRustError> {
    let metadata = fs::symlink_metadata(lockfile)?;
    if !metadata.is_file() || metadata.file_type().is_symlink()
        || metadata.len() > 8 * 1024 * 1024
    {
        return Err(VerifiedRustError::Dependency("Git dependency lockfile invalid".into()));
    }
    let cargo: toml::Value = toml::from_str(&fs::read_to_string(lockfile)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    let entries = cargo.get("package").and_then(toml::Value::as_array)
        .ok_or_else(|| VerifiedRustError::Dependency("Cargo.lock has no packages".into()))?;
    let bare = repo.clone_url.trim_end_matches(".git");
    let candidates = entries.iter().filter_map(|entry| {
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
    }).collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err(VerifiedRustError::Dependency(
            "Cargo.lock did not uniquely resolve requested exact Git revision".into()
        ));
    }
    Ok(candidates[0].to_owned())
}

/// Publish a new target copy ONLY if a reviewed Git checkout and the Cargo
/// lockfile point to the exact same commit, and target-owned tests pass in an
/// offline, resource-bounded Docker container.
pub fn verify_upstream_git(options: &UpstreamGitOptions) -> Result<UpstreamGitResult, VerifiedRustError> {
    if !options.license_reviewed {
        return Err(VerifiedRustError::ReviewRequired);
    }
    let repo = GitHubRepository::parse(&options.upstream_repository)
        .map_err(|_| VerifiedRustError::Unsafe("canonical public HTTPS GitHub URL required".into()))?;
    let rev = &options.upstream_commit;
    if rev.len() != 40
        || !rev.bytes().all(|c| c.is_ascii_hexdigit())
        || !valid_package(&options.package)
        || !valid_public_path(&options.public_symbol)
        || !ascii_ident(&options.target_alias)
    {
        return Err(VerifiedRustError::Unsafe(
            "Git SHA, package, public symbol or target alias invalid".into()
        ));
    }
    let (license, license_sha) = checkout_license(
        &options.upstream_checkout, &options.package, &repo, rev,
    )?;
    let original = options.target_root.canonicalize()?;
    validate_contract_test(&original, &options.contract_test, &options.target_alias)?;
    let target_manifest = original.join("Cargo.toml");
    let mut cargo: toml::Value = toml::from_str(&fs::read_to_string(&target_manifest)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    if cargo.get("package").is_none() || cargo.get("workspace").is_some()
        || cargo.get("patch").is_some() || cargo.get("replace").is_some()
    {
        return Err(VerifiedRustError::Dependency(
            "Git capability import supports only standalone, unpatched Cargo targets".into()
        ));
    }
    let lib = original.join("src/lib.rs");
    let meta = fs::symlink_metadata(&lib)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe("target library file is unsafe".into()));
    }
    let original_lib = fs::read_to_string(&lib)?;
    let dependency_key = format!("composer_git_{}", options.package.replace('-', "_"));
    let dependencies = cargo
        .as_table_mut()
        .ok_or_else(|| VerifiedRustError::Toml("target manifest is not a table".into()))?
        .entry("dependencies".to_owned())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| VerifiedRustError::Dependency("target dependencies are not a table".into()))?;
    if dependencies.contains_key(&dependency_key)
        || original_lib.contains(&format!("pub use {dependency_key}::"))
    {
        return Err(VerifiedRustError::Unsafe(
            "Git dependency alias already exists in target".into()
        ));
    }
    let mut dependency = toml::map::Map::new();
    dependency.insert("package".into(), toml::Value::String(options.package.clone()));
    dependency.insert("git".into(), toml::Value::String(repo.clone_url.clone()));
    dependency.insert("rev".into(), toml::Value::String(rev.clone()));
    dependencies.insert(dependency_key.clone(), toml::Value::Table(dependency));
    let destination = &options.output_root;
    if destination.exists()
        || !matches!(destination.components().next_back(), Some(Component::Normal(_)))
    {
        return Err(VerifiedRustError::OutputExists(destination.clone()));
    }
    let parent = destination.parent().ok_or_else(|| {
        VerifiedRustError::Unsafe("Git output requires an existing parent".into())
    })?.canonicalize()?;
    let name = destination.file_name().ok_or_else(|| {
        VerifiedRustError::Unsafe("Git output requires a final name".into())
    })?;
    let output = parent.join(name);
    if output.starts_with(&original) || original.starts_with(&output) {
        return Err(VerifiedRustError::Unsafe("output overlaps original target".into()));
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
        copy_bounded_target(&original, &staged)?;
        fs::write(
            staged.join("Cargo.toml"),
            toml::to_string_pretty(&cargo)
                .map_err(|err| VerifiedRustError::Toml(err.to_string()))?,
        )?;
        let mut library = original_lib.clone();
        library.push_str(&format!(
            "\n// Composer: reused pinned upstream Git capability, not regenerated\n\
             pub use {dependency_key}::{} as {};\n",
            options.public_symbol, options.target_alias,
        ));
        fs::write(staged.join("src/lib.rs"), library)?;
        let image_id = inspect_local_image_id()?;
        let container_user = nonroot_container_user()?;
        let start = Instant::now();
        let logs = docker_build_with_git_cache(
            &staged, &results, Some(&options.registry_cache),
            &options.git_cache, &image_id, &container_user,
        )?;
        let elapsed = start.elapsed().as_secs_f64();
        let source = validate_git_lock(
            &results.join("Cargo.lock"), &options.package, &repo, rev,
        )?;
        fs::copy(results.join("Cargo.lock"), staged.join("Cargo.lock"))?;
        let licenses = staged.join("third_party_licenses");
        fs::create_dir_all(&licenses)?;
        let notice = format!("{}-git-license.txt", options.package);
        fs::write(licenses.join(&notice), &license)?;
        let evidence = serde_json::json!({
            "schema_version": 1,
            "mode": "reuse_pinned_git_dependency",
            "source_repo": repo.clone_url,
            "source_commit": rev,
            "lockfile_source": source,
            "source_checkout_commit_verified": true,
            "cargo_lock_commit_verified": true,
            "license_sha256": license_sha,
            "license_notice": format!("third_party_licenses/{notice}"),
            "review_attested": true,
            "target_contract": options.contract_test,
            "public_symbol": options.public_symbol,
            "target_alias": options.target_alias,
            "offline_tests_passed": true,
            "network_disabled": true,
            "docker_image_id": image_id,
            "original_target_unchanged": true,
            "elapsed_seconds": elapsed,
            "log_tail": logs,
            "limitations": "Reviewed Git commit pinned by Cargo and checked against original clean checkout and Cargo.lock; independent target tests are limited and Docker is not VM-grade isolation."
        });
        fs::write(staged.join(GIT_EVIDENCE), serde_json::to_vec_pretty(&evidence)?)?;
        fs::rename(&staged, &output)?;
        Ok(UpstreamGitResult {
            output_root: output.clone(),
            package: options.package.clone(),
            public_symbol: options.public_symbol.clone(),
            target_alias: options.target_alias.clone(),
            upstream_repository: repo.clone_url.clone(),
            upstream_commit: rev.clone(),
            lockfile_git_commit_verified: true,
            source_checkout_commit_verified: true,
            source_license_sha256: license_sha.clone(),
            original_target_unchanged: true,
            offline_tests_passed: true,
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
            contract_test: PathBuf::from("tests/contract.rs"),
            registry_cache: PathBuf::from("missing-registry"),
            git_cache: PathBuf::from("missing-git"),
            license_reviewed: false,
        };
        assert!(matches!(verify_upstream_git(&opts), Err(VerifiedRustError::ReviewRequired)));
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
