//! Opt-in reuse of an existing, version-pinned public Rust crate.
//!
//! Cargo provides dependency acquisition/resolution and the upstream crate
//! provides the implementation: Composer does not regenerate its source.
//! Nothing from the external crate is built or executed on the host.
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::Instant,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    VerifiedRustError,
    reviewed::validate_contract_test,
    verified::{copy_bounded_target, docker_build, inspect_local_image_id, nonroot_container_user},
};

const EVIDENCE_FILE: &str = "composer-upstream-crate-evidence.json";

#[derive(Debug, Clone)]
pub struct UpstreamCrateOptions {
    pub target_root: PathBuf,
    pub output_root: PathBuf,
    pub package: String,
    pub version: String,
    pub public_symbol: String,
    pub target_alias: String,
    pub contract_test: PathBuf,
    pub registry_cache: PathBuf,
    /// Human-reviewed provenance assertion, NOT automatic source/release matching.
    pub upstream_repository: String,
    pub upstream_commit: String,
    pub license_reviewed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamCrateResult {
    pub output_root: PathBuf,
    pub package: String,
    pub locked_version: String,
    pub public_symbol: String,
    pub target_alias: String,
    pub registry_checksum: String,
    pub upstream_repository: String,
    pub upstream_commit_claimed: String,
    pub provenance_commit_matched_to_registry_artifact: bool,
    pub original_target_unchanged: bool,
    pub offline_tests_passed: bool,
    pub evidence_path: PathBuf,
    pub elapsed_seconds: f64,
}

fn rust_identifier(value: &str) -> bool {
    !value.is_empty()
        && value != "_"
        && value.len() <= 80
        && value.as_bytes()[0].is_ascii_alphabetic()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn package_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-')
}

fn validated_upstream_repository(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://github.com/") else {
        return false;
    };
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let parts = rest.split('/').collect::<Vec<_>>();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 100
                && !part.starts_with('.')
                && !part.ends_with('.')
                && part.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                })
        })
}

fn locked_registry_checksum(lockfile: &Path, package: &str, version: &str) -> Result<String, VerifiedRustError> {
    let lock: toml::Value = toml::from_str(&fs::read_to_string(lockfile)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    let packages = lock.get("package").and_then(toml::Value::as_array)
        .ok_or_else(|| VerifiedRustError::Dependency("resolved lockfile has no packages".into()))?;
    let matched = packages.iter().filter(|entry| {
        entry.get("name").and_then(toml::Value::as_str) == Some(package)
            && entry.get("version").and_then(toml::Value::as_str) == Some(version)
            && entry.get("source").and_then(toml::Value::as_str)
                == Some("registry+https://github.com/rust-lang/crates.io-index")
    }).collect::<Vec<_>>();
    if matched.len() != 1 {
        return Err(VerifiedRustError::Dependency(
            format!("pinned crates.io dependency {package} {version} not uniquely resolved in Cargo.lock")
        ));
    }
    let checksum = matched[0].get("checksum").and_then(toml::Value::as_str)
        .ok_or_else(|| VerifiedRustError::Dependency("registry package lock checksum is missing".into()))?;
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(VerifiedRustError::Dependency(
            "registry package lock checksum is invalid".into()
        ));
    }
    Ok(checksum.to_owned())
}

/// Publish a new target copy ONLY after Cargo's offline sandbox has compiled
/// the exact released upstream crate and run target-owned behavioral tests.
/// Caller must independently review licenses and selected upstream capabilities.
pub fn verify_upstream_crate(
    options: &UpstreamCrateOptions,
) -> Result<UpstreamCrateResult, VerifiedRustError> {
    if !options.license_reviewed {
        return Err(VerifiedRustError::ReviewRequired);
    }
    if !package_identifier(&options.package)
        || !rust_identifier(&options.public_symbol)
        || !rust_identifier(&options.target_alias)
        || !validated_upstream_repository(&options.upstream_repository)
        || options.upstream_commit.len() != 40
        || !options.upstream_commit.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(VerifiedRustError::Unsafe(
            "invalid crate, API symbol, alias, upstream GitHub URL or 40-digit upstream commit".into()
        ));
    }
    let exact = semver::Version::parse(&options.version)
        .map_err(|err| VerifiedRustError::Unsafe(format!("exact crate version required: {err}")))?;
    if exact.to_string() != options.version {
        return Err(VerifiedRustError::Unsafe("crate version must be canonical and exact".into()));
    }

    let source = options.target_root.canonicalize()?;
    // The reviewed contract is supplied by the target, not generated from
    // upstream source. Avoid accepting a test that Cargo could skip.
    validate_contract_test(&source, &options.contract_test, &options.target_alias)?;
    let manifest = source.join("Cargo.toml");
    let toml_text = fs::read_to_string(&manifest)?;
    let mut cargo: toml::Value = toml::from_str(&toml_text)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    if cargo.get("package").is_none() || cargo.get("workspace").is_some() {
        return Err(VerifiedRustError::Dependency(
            "upstream crate adoption requires a standalone Cargo package".into()
        ));
    }
    let lib = source.join("src/lib.rs");
    let lib_meta = fs::symlink_metadata(&lib)?;
    if !lib_meta.is_file() || lib_meta.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe(
            "target requires regular src/lib.rs".into()
        ));
    }
    let lib_source = fs::read_to_string(&lib)?;
    if lib_source.contains(&format!("pub fn {}", options.target_alias))
        || lib_source.contains(&format!("pub use {}", options.package.replace('-', "_")))
    {
        return Err(VerifiedRustError::Unsafe("target alias or upstream export may collide".into()));
    }
    // Prevent silently rewriting or shadowing existing dependency requirements.
    for group in ["dependencies", "build-dependencies", "dev-dependencies"] {
        if cargo.get(group)
            .and_then(toml::Value::as_table)
            .is_some_and(|table| table.contains_key(&options.package))
        {
            return Err(VerifiedRustError::Dependency(
                format!("target already declares {}; resolve the dependency explicitly first", options.package)
            ));
        }
    }
    if cargo.get("patch").is_some() || cargo.get("replace").is_some() {
        return Err(VerifiedRustError::Dependency(
            "targets with custom Cargo [patch]/[replace] registries are unsupported".into()
        ));
    }
    let values = cargo.as_table_mut()
        .ok_or_else(|| VerifiedRustError::Toml("target manifest must be a table".into()))?;
    let dependencies = values
        .entry("dependencies")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| VerifiedRustError::Dependency("target [dependencies] is invalid".into()))?;
    dependencies.insert(
        options.package.clone(),
        toml::Value::String(format!("={}", options.version)),
    );

    let dest = &options.output_root;
    if dest.exists() {
        return Err(VerifiedRustError::OutputExists(dest.clone()));
    }
    if !matches!(dest.components().next_back(), Some(Component::Normal(_))) {
        return Err(VerifiedRustError::Unsafe("output must be a new, normal directory".into()));
    }
    let parent = dest.parent().ok_or_else(|| VerifiedRustError::Unsafe(
        "output requires an existing parent directory".into()
    ))?.canonicalize()?;
    let name = dest.file_name().ok_or_else(|| VerifiedRustError::Unsafe(
        "output requires a final name".into()
    ))?;
    let output = parent.join(name);
    if output.starts_with(&source) || source.starts_with(&output) {
        return Err(VerifiedRustError::Unsafe(
            "output and original target must be separate directories".into()
        ));
    }

    let id = Uuid::new_v4();
    let staged = parent.join(format!(".composer-upstream-{id}.tmp"));
    let results = parent.join(format!(".composer-upstream-results-{id}.tmp"));
    fs::create_dir(&staged)?;
    if let Err(err) = fs::create_dir(&results) {
        let _ = fs::remove_dir_all(&staged);
        return Err(err.into());
    }

    let outcome = (|| {
        copy_bounded_target(&source, &staged)?;
        let edited = toml::to_string_pretty(&cargo)
            .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
        fs::write(staged.join("Cargo.toml"), edited)?;
        let mut updated_lib = lib_source.clone();
        updated_lib.push_str(&format!(
            "\n// Composer: reviewed published crate reuse (no source regeneration)\n\
             pub use {}::{} as {};\n",
            options.package.replace('-', "_"),
            options.public_symbol,
            options.target_alias
        ));
        fs::write(staged.join("src/lib.rs"), updated_lib)?;
        let image_id = inspect_local_image_id()?;
        let user = nonroot_container_user()?;
        let started = Instant::now();
        let logs = docker_build(
            &staged, &results, Some(&options.registry_cache), &image_id, &user,
        )?;
        let elapsed = started.elapsed().as_secs_f64();
        let lockfile = results.join("Cargo.lock");
        let locked_checksum = locked_registry_checksum(
            &lockfile, &options.package, &options.version,
        )?;
        fs::copy(&lockfile, staged.join("Cargo.lock"))?;
        let evidence = serde_json::json!({
            "schema_version": 1,
            "mode": "reuse_published_third_party_crate",
            "package": options.package,
            "pinned_version": options.version,
            "registry_checksum": locked_checksum,
            "public_symbol": options.public_symbol,
            "target_alias": options.target_alias,
            "contract_test": options.contract_test,
            "upstream_repository_claimed": options.upstream_repository,
            "upstream_commit_claimed": options.upstream_commit,
            "provenance_commit_matched_to_registry_artifact": false,
            "human_license_review_attested": true,
            "docker_image_id": image_id,
            "network_disabled": true,
            "original_target_unchanged": true,
            "offline_tests_passed": true,
            "elapsed_seconds": elapsed,
            "log_tail": logs,
            "limitations": "Registry artifact is verified by Cargo checksum; supplied GitHub commit is metadata, not cryptographically matched to the released archive. License clearance and complete semantic equivalence are not certified."
        });
        fs::write(staged.join(EVIDENCE_FILE), serde_json::to_vec_pretty(&evidence)?)?;
        fs::rename(&staged, &output)?;
        Ok(UpstreamCrateResult {
            output_root: output.clone(),
            package: options.package.clone(),
            locked_version: options.version.clone(),
            public_symbol: options.public_symbol.clone(),
            target_alias: options.target_alias.clone(),
            registry_checksum: locked_checksum,
            upstream_repository: options.upstream_repository.clone(),
            upstream_commit_claimed: options.upstream_commit.clone(),
            provenance_commit_matched_to_registry_artifact: false,
            original_target_unchanged: true,
            offline_tests_passed: true,
            evidence_path: output.join(EVIDENCE_FILE),
            elapsed_seconds: elapsed,
        })
    })();
    let _ = fs::remove_dir_all(&results);
    if outcome.is_err() {
        let _ = fs::remove_dir_all(&staged);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_pinned_or_injected_upstream_inputs() {
        for value in ["^1.0", "1", ">=1.0", "1.0.0;pwd"] {
            assert!(semver::Version::parse(value).is_err());
        }
        for value in ["strsim;sh", "owner/repo", "../strsim", ""] {
            assert!(!package_identifier(value));
        }
        for value in ["https://evil.invalid/owner/repo",
            "https://github.com/owner/repo/tree/main",
            "https://github.com/owner/repo?token=x"] {
            assert!(!validated_upstream_repository(value));
        }
        assert!(validated_upstream_repository("https://github.com/rapidfuzz/strsim-rs"));
        assert!(package_identifier("strsim"));
        assert!(rust_identifier("jaro"));
        assert!(rust_identifier("ported_jaro"));
    }

    #[test]
    fn fails_without_license_review_before_touching_target() {
        let opts = UpstreamCrateOptions {
            target_root: PathBuf::from("does-not-exist"),
            output_root: PathBuf::from("should-not-exist"),
            package: "strsim".into(), version: "0.11.1".into(),
            public_symbol: "jaro".into(), target_alias: "ported_jaro".into(),
            contract_test: PathBuf::from("tests/contract.rs"),
            registry_cache: PathBuf::from("cache"),
            upstream_repository: "https://github.com/rapidfuzz/strsim-rs".into(),
            upstream_commit: "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26".into(),
            license_reviewed: false,
        };
        assert!(matches!(verify_upstream_crate(&opts), Err(VerifiedRustError::ReviewRequired)));
    }
}
