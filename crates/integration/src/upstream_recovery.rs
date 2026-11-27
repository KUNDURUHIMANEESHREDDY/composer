//! Bounded fallback across independently reviewed upstream package candidates.
//! Relies on Cargo's existing registry and resolution, never regenerates algorithms.
//! Each attempt invokes the existing offline Docker verifier, not host execution.
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    UpstreamCrateOptions, VerifiedRustError, verify_upstream_crate,
};

const MAX_CANDIDATES: usize = 5;
const MAX_ATTEMPTS: usize = 10;
const MAX_VERSIONS_PER_CANDIDATE: usize = 3;
const RECOVERY_EVIDENCE: &str = "composer-recovery-evidence.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamRecoveryCandidate {
    pub package: String,
    /// Exact release, semver constraint (e.g. ^0.11), or latest *cached* stable.
    pub version_requirement: String,
    /// Public API path relative to this published crate, e.g. str::jaro.
    pub public_symbol: String,
    /// Independently reviewed origin metadata (not cryptographically linked to .crate).
    pub upstream_repository: String,
    pub upstream_commit: String,
}

#[derive(Debug, Clone)]
pub struct UpstreamRecoveryOptions {
    pub target_root: PathBuf,
    pub output_root: PathBuf,
    pub target_alias: String,
    pub contract_test: PathBuf,
    pub registry_cache: PathBuf,
    pub candidates: Vec<UpstreamRecoveryCandidate>,
    pub license_reviewed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamRecoveryAttempt {
    pub package: String,
    pub version: Option<String>,
    pub public_symbol: String,
    pub isolated_dependency: bool,
    pub status: String,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamRecoveryResult {
    pub completed: bool,
    pub output_root: Option<PathBuf>,
    pub selected_package: Option<String>,
    pub selected_version: Option<String>,
    pub selected_symbol: Option<String>,
    pub tests_passed: bool,
    pub original_target_unchanged: bool,
    pub attempts: Vec<UpstreamRecoveryAttempt>,
    pub evidence_path: Option<PathBuf>,
    pub limitation: String,
}

fn version_constraint(input: &str) -> Result<VersionReq, VerifiedRustError> {
    if input == "latest" {
        return Ok(VersionReq::STAR);
    }
    if let Ok(version) = Version::parse(input) {
        return VersionReq::parse(&format!("={version}")).map_err(|err| {
            VerifiedRustError::Dependency(format!("invalid exact version: {err}"))
        });
    }
    VersionReq::parse(input)
        .map_err(|err| VerifiedRustError::Dependency(format!("invalid semver range: {err}")))
}

fn registry_versions(cache: &Path, package: &str) -> Result<Vec<Version>, VerifiedRustError> {
    if package.is_empty()
        || package.len() > 64
        || !package.bytes().all(|byte| byte.is_ascii_lowercase()
            || byte.is_ascii_digit() || byte == b'-' || byte == b'_')
    {
        return Err(VerifiedRustError::Unsafe("invalid registry package".into()));
    }
    let dir = cache.join("cache");
    let meta = fs::symlink_metadata(&dir).map_err(|_| {
        VerifiedRustError::Dependency("Cargo registry cache/cache is not provisioned".into())
    })?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe("Cargo cache directory must be regular".into()));
    }
    let mut versions = BTreeSet::new();
    let registries = fs::read_dir(&dir)?.collect::<Result<Vec<_>, _>>()?;
    if registries.len() > 16 {
        return Err(VerifiedRustError::Unsafe("too many registry cache roots".into()));
    }
    let prefix = format!("{package}-");
    for registry in registries {
        let name = registry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("index.crates.io-") || !registry.file_type()?.is_dir() {
            continue;
        }
        let entries = fs::read_dir(registry.path())?.collect::<Result<Vec<_>, _>>()?;
        if entries.len() > 30000 {
            return Err(VerifiedRustError::Unsafe("Cargo cache entry limit exceeded".into()));
        }
        for entry in entries {
            let filename = entry.file_name().to_string_lossy().into_owned();
            let Some(version_string) = filename
                .strip_prefix(&prefix)
                .and_then(|v| v.strip_suffix(".crate"))
            else {
                continue;
            };
            if !entry.file_type()?.is_file() {
                continue;
            }
            if let Ok(version) = Version::parse(version_string)
                && version.pre.is_empty()
            {
                versions.insert(version);
            }
        }
    }
    let mut versions = versions.into_iter().collect::<Vec<_>>();
    versions.reverse();
    Ok(versions)
}

fn existing_direct_dependency(target: &Path, package: &str) -> Result<bool, VerifiedRustError> {
    let cargo: toml::Value = toml::from_str(&fs::read_to_string(target.join("Cargo.toml"))?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    Ok(["dependencies", "build-dependencies", "dev-dependencies"].iter().any(|key| {
        cargo.get(*key).and_then(toml::Value::as_table)
            .is_some_and(|table| table.contains_key(package))
    }))
}

fn short_error(err: &VerifiedRustError) -> String {
    let kind = match err {
        VerifiedRustError::Docker(_) => "docker_verification",
        VerifiedRustError::Dependency(_) => "dependency_constraint",
        VerifiedRustError::Unsafe(_) => "safety_gate",
        _ => "integration_gate",
    };
    let summary = err.to_string().chars().take(360).collect::<String>();
    format!("{kind}: {summary}")
}

fn version_plan(
    options: &UpstreamRecoveryOptions,
    candidate: &UpstreamRecoveryCandidate,
) -> Result<Vec<Version>, VerifiedRustError> {
    let req = version_constraint(&candidate.version_requirement)?;
    let versions = registry_versions(&options.registry_cache, &candidate.package)?;
    let matches = versions
        .into_iter()
        .filter(|version| req.matches(version))
        .take(MAX_VERSIONS_PER_CANDIDATE)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Err(VerifiedRustError::Dependency(format!(
            "no stable, pre-cached crates.io release matches {} {} (no network fetch attempted)",
            candidate.package, candidate.version_requirement
        )));
    }
    Ok(matches)
}

/// Try alternatives in stated preference order. All source and license review
/// is explicit, and no candidate may change the original target. A failed
/// behavioral assertion rejects that upstream before another is tried.
/// Incompatible existing versions may coexist via a namespaced Cargo alias.
/// This does not search GitHub itself: inputs are curated third-party candidates.
pub fn recover_upstream_crate(
    options: &UpstreamRecoveryOptions,
) -> Result<UpstreamRecoveryResult, VerifiedRustError> {
    if !options.license_reviewed {
        return Err(VerifiedRustError::ReviewRequired);
    }
    if !(1..=MAX_CANDIDATES).contains(&options.candidates.len()) {
        return Err(VerifiedRustError::Unsafe(
            "provide 1-5 independently reviewed upstream candidates".into(),
        ));
    }
    if options.output_root.exists() {
        return Err(VerifiedRustError::OutputExists(options.output_root.clone()));
    }
    if !matches!(
        options.output_root.components().next_back(),
        Some(Component::Normal(_))
    ) {
        return Err(VerifiedRustError::Unsafe(
            "recovery output must be a new normal directory".into(),
        ));
    }
    let original = options.target_root.canonicalize()?;
    let parent = options.output_root.parent()
        .ok_or_else(|| VerifiedRustError::Unsafe("output parent required".into()))?
        .canonicalize()?;
    let name = options.output_root.file_name()
        .ok_or_else(|| VerifiedRustError::Unsafe("output name required".into()))?;
    let final_output = parent.join(name);
    if final_output.starts_with(&original) || original.starts_with(&final_output) {
        return Err(VerifiedRustError::Unsafe(
            "output must not overlap original target".into(),
        ));
    }
    // Fail early for invalid target-owned contracts, rather than classifying
    // them as unsuccessful implementations during candidate fallback.
    crate::reviewed::validate_contract_test(
        &original, &options.contract_test, &options.target_alias,
    )?;

    let mut attempts = Vec::new();
    for candidate in &options.candidates {
        if attempts.len() >= MAX_ATTEMPTS {
            break;
        }
        let versions = match version_plan(options, candidate) {
            Ok(versions) => versions,
            Err(err) => {
                attempts.push(UpstreamRecoveryAttempt {
                    package: candidate.package.clone(),
                    version: None,
                    public_symbol: candidate.public_symbol.clone(),
                    isolated_dependency: false,
                    status: "unavailable_or_invalid_candidate".into(),
                    explanation: short_error(&err),
                });
                continue;
            }
        };
        let conflict = existing_direct_dependency(&original, &candidate.package)?;
        for version in versions {
            if attempts.len() >= MAX_ATTEMPTS {
                break;
            }
            let alias = conflict.then(|| format!("composer_{}", candidate.package.replace('-', "_")));
            let scratch = parent.join(format!(".composer-recovery-{}.tmp", Uuid::new_v4()));
            let result = verify_upstream_crate(&UpstreamCrateOptions {
                target_root: original.clone(),
                output_root: scratch.clone(),
                package: candidate.package.clone(),
                version: version.to_string(),
                public_symbol: candidate.public_symbol.clone(),
                target_alias: options.target_alias.clone(),
                dependency_alias: alias.clone(),
                contract_test: options.contract_test.clone(),
                registry_cache: options.registry_cache.clone(),
                upstream_repository: candidate.upstream_repository.clone(),
                upstream_commit: candidate.upstream_commit.clone(),
                license_reviewed: true,
            });
            match result {
                Ok(verified) => {
                    attempts.push(UpstreamRecoveryAttempt {
                        package: candidate.package.clone(),
                        version: Some(version.to_string()),
                        public_symbol: candidate.public_symbol.clone(),
                        isolated_dependency: conflict,
                        status: "verified".into(),
                        explanation: "Independent target tests and Cargo lockfile verified in offline Docker".into(),
                    });
                    let proof_path = scratch.join(RECOVERY_EVIDENCE);
                    let proof = serde_json::json!({
                        "schema_version": 1,
                        "selected_package": candidate.package,
                        "selected_version": version.to_string(),
                        "selected_symbol": candidate.public_symbol,
                        "isolated_dependency": conflict,
                        "registry_checksum": verified.registry_checksum,
                        "attempts": attempts,
                        "network_disabled": true,
                        "target_unchanged": true,
                        "source_repository_claimed": candidate.upstream_repository,
                        "source_commit_claimed": candidate.upstream_commit,
                        "provenance_commit_matched_to_registry_artifact": false,
                        "tests_passed": true,
                    });
                    if let Err(err) = fs::write(&proof_path, serde_json::to_vec_pretty(&proof)?) {
                        let _ = fs::remove_dir_all(&scratch);
                        return Err(err.into());
                    }
                    if let Err(err) = fs::rename(&scratch, &final_output) {
                        let _ = fs::remove_dir_all(&scratch);
                        return Err(err.into());
                    }
                    return Ok(UpstreamRecoveryResult {
                        completed: true,
                        output_root: Some(final_output.clone()),
                        selected_package: Some(candidate.package.clone()),
                        selected_version: Some(version.to_string()),
                        selected_symbol: Some(candidate.public_symbol.clone()),
                        tests_passed: true,
                        original_target_unchanged: true,
                        attempts,
                        evidence_path: Some(final_output.join(RECOVERY_EVIDENCE)),
                        limitation: "Candidates require independent review; no online repository discovery or provenance matching. Docker is not VM isolation.".into(),
                    });
                }
                Err(err) => {
                    // verify_upstream_crate publishes scratch only after tests
                    // pass. Defend against partial artifacts anyway.
                    let _ = fs::remove_dir_all(&scratch);
                    attempts.push(UpstreamRecoveryAttempt {
                        package: candidate.package.clone(),
                        version: Some(version.to_string()),
                        public_symbol: candidate.public_symbol.clone(),
                        isolated_dependency: conflict,
                        status: "rejected".into(),
                        explanation: short_error(&err),
                    });
                }
            }
        }
    }
    Ok(UpstreamRecoveryResult {
        completed: false,
        output_root: None,
        selected_package: None,
        selected_version: None,
        selected_symbol: None,
        tests_passed: false,
        original_target_unchanged: true,
        attempts,
        evidence_path: None,
        limitation: "No reviewed, cached alternative passed independent tests; original target untouched. Candidate discovery and provenance matching remain external.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_newest_cached_stable_version_and_semver_range() {
        let base = std::env::temp_dir().join(format!("composer-cache-{}", Uuid::new_v4()));
        let cache = base.join("cache/index.crates.io-test");
        fs::create_dir_all(&cache).unwrap();
        for name in ["strsim-0.10.0.crate", "strsim-0.11.0.crate",
                     "strsim-0.11.1.crate", "strsim-0.12.0-alpha.1.crate",
                     "unrelated-9.9.9.crate"] {
            fs::write(cache.join(name), b"fixture").unwrap();
        }
        let versions = registry_versions(&base, "strsim").unwrap();
        assert_eq!(versions[0], Version::parse("0.11.1").unwrap());
        assert_eq!(versions.len(), 3);
        let req = version_constraint("^0.11").unwrap();
        assert_eq!(
            versions.into_iter().filter(|v| req.matches(v)).collect::<Vec<_>>(),
            vec![Version::parse("0.11.1").unwrap(), Version::parse("0.11.0").unwrap()],
        );
        let exact = version_constraint("0.11.1").unwrap();
        assert!(!exact.matches(&Version::parse("0.11.0").unwrap()));
        assert!(version_constraint("latest").unwrap().matches(&Version::parse("0.11.1").unwrap()));
        assert!(version_constraint("not a version!").is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn never_runs_unreviewed_recovery_or_accepts_unbounded_candidates() {
        let options = UpstreamRecoveryOptions {
            target_root: PathBuf::from("not-present"),
            output_root: PathBuf::from("should-not-exist"),
            target_alias: "ported_jaro".into(),
            contract_test: PathBuf::from("tests/external_contract.rs"),
            registry_cache: PathBuf::from("missing"),
            candidates: vec![],
            license_reviewed: false,
        };
        assert!(matches!(
            recover_upstream_crate(&options),
            Err(VerifiedRustError::ReviewRequired),
        ));
        let options = UpstreamRecoveryOptions {
            license_reviewed: true,
            ..options
        };
        assert!(matches!(
            recover_upstream_crate(&options),
            Err(VerifiedRustError::Unsafe(_)),
        ));
    }
}
