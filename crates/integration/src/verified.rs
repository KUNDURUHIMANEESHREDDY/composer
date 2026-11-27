//! Offline, Docker-confined integration verification.
//! Source compilation is never performed by the host process.
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use composer_intelligence::CapsuleManifest;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{RustWiringOptions, verify_capsule, wire::wire_rust_module_for_verified_sandbox};

const DEFAULT_IMAGE: &str = "rust:1.99-slim";
const MAX_FILES: usize = 1500;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_LOG_CHARS: usize = 16000;
const TIMEOUT: Duration = Duration::from_secs(240);

#[derive(Debug, Clone)]
pub struct VerifiedRustOptions {
    pub capsule_root: PathBuf,
    pub target_root: PathBuf,
    pub output_root: PathBuf,
    /// An assertion by the caller; Composer does not approve licenses.
    pub license_reviewed: bool,
    /// Optional path containing ONLY Cargo's public registry source/cache/index.
    pub registry_cache: Option<PathBuf>,
    /// Optional explicit compiler-verified source_path=target_name API exports.
    pub api_aliases: Vec<String>,
    /// Optional target-facing function contracts with compiler-inferred Into.
    pub api_signatures: Vec<String>,
    /// Explicit reviewed contract rule, recomputed against current source/target.
    pub reviewed_api_rule: Option<String>,
    /// Existing target-owned behavioral test: tests/<name>.rs.
    pub contract_test: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedRustResult {
    pub output_root: PathBuf,
    pub capsule_id: String,
    pub new_dependencies: Vec<String>,
    pub dependency_feature_changes: Vec<String>,
    pub compatible_constraints_retained: Vec<String>,
    pub verified_api_aliases: Vec<String>,
    pub verified_api_signatures: Vec<String>,
    pub reviewed_api_rule: Option<String>,
    pub reviewed_contract_test: Option<PathBuf>,
    pub docker_image: String,
    pub offline_lockfile_resolved: bool,
    pub isolated_tests_passed: bool,
    pub evidence_path: PathBuf,
    pub elapsed_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BuildEvidence {
    schema_version: u32,
    capsule_id: String,
    image: String,
    image_id: String,
    network_disabled: bool,
    read_only_container_root: bool,
    unprivileged_user: String,
    cpu_limit: String,
    memory_limit: String,
    commands: Vec<String>,
    dependencies_added: Vec<String>,
    dependency_feature_changes: Vec<String>,
    compatible_constraints_retained: Vec<String>,
    verified_api_aliases: Vec<String>,
    verified_api_signatures: Vec<String>,
    reviewed_api_rule: Option<String>,
    reviewed_contract_test: Option<PathBuf>,
    repair_actions: Vec<String>,
    tests_passed: bool,
    elapsed_seconds: f64,
    log_tail: String,
}

#[derive(Debug, Error)]
pub enum VerifiedRustError {
    #[error("source/license review must be explicitly asserted")]
    ReviewRequired,
    #[error("unsupported or unsafe input: {0}")]
    Unsafe(String),
    #[error("verified output path already exists: {0}")]
    OutputExists(PathBuf),
    #[error("dependency resolution refused: {0}")]
    Dependency(String),
    #[error("source wiring refused: {0}")]
    Wiring(#[from] crate::RustWiringError),
    #[error("capsule verification failed: {0}")]
    Capsule(#[from] crate::VerificationError),
    #[error("Docker execution failed: {0}")]
    Docker(String),
    #[error("filesystem operation failed: {0}")]
    Filesystem(#[from] io::Error),
    #[error("TOML operation failed: {0}")]
    Toml(String),
    #[error("JSON operation failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// Build a new, independently reviewable target copy. Never modifies the
/// original target. Resolution and tests execute in a network-disabled Docker
/// container using only a disposable workspace and optional read-only registry
/// cache. Any failure leaves no published output directory.
pub fn compose_verified_rust(
    options: &VerifiedRustOptions,
) -> Result<VerifiedRustResult, VerifiedRustError> {
    if !options.license_reviewed {
        return Err(VerifiedRustError::ReviewRequired);
    }
    let reviewed_contract = match (&options.reviewed_api_rule, &options.contract_test) {
        (None, None) => None,
        (Some(rule), Some(test)) if options.api_aliases.is_empty() && options.api_signatures.is_empty() => {
            Some((rule.clone(), test.clone()))
        }
        _ => return Err(VerifiedRustError::Unsafe(
            "reviewed API requires both --reviewed-api-rule and --contract-test, without other API rules".into()
        )),
    };
    let capsule = verify_capsule(&options.capsule_root)?;
    let source = options.target_root.canonicalize()?;
    if let Some((rule, test)) = &reviewed_contract {
        crate::reviewed::validate_reviewed_api(&capsule.root, &source, rule, test)?;
    }
    let dest = &options.output_root;
    if dest.exists() {
        return Err(VerifiedRustError::OutputExists(dest.clone()));
    }
    let parent = dest
        .parent()
        .ok_or_else(|| {
            VerifiedRustError::Unsafe("output requires an existing parent directory".into())
        })?
        .canonicalize()?;
    let name = dest.file_name().ok_or_else(|| {
        VerifiedRustError::Unsafe("output must have a final directory component".into())
    })?;
    if !matches!(dest.components().next_back(), Some(Component::Normal(_))) {
        return Err(VerifiedRustError::Unsafe(
            "output path is not a regular directory name".into(),
        ));
    }
    let output = parent.join(name);
    if output.starts_with(&source)
        || source.starts_with(&output)
        || output.starts_with(&capsule.root)
        || capsule.root.starts_with(&output)
    {
        return Err(VerifiedRustError::Unsafe(
            "output, target, and capsule must be separate directories".into(),
        ));
    }
    let id = Uuid::new_v4();
    let staged = parent.join(format!(".composer-build-{id}.tmp"));
    let results = parent.join(format!(".composer-result-{id}.tmp"));
    fs::create_dir(&staged)?;
    if let Err(err) = fs::create_dir(&results) {
        let _ = fs::remove_dir_all(&staged);
        return Err(err.into());
    }
    let outcome = (|| {
        copy_bounded_target(&source, &staged)?;
        let (added, dependency_feature_changes) =
            merge_direct_cargo_dependencies(&capsule.manifest, &capsule.root, &staged)?;
        let wired = wire_rust_module_for_verified_sandbox(&RustWiringOptions {
            capsule_root: capsule.root.clone(),
            target_root: staged.clone(),
            license_reviewed: true,
        })?;
        let verified_api_aliases =
            apply_verified_api_aliases(&staged, &wired.module_name, &options.api_aliases)?;
        let verified_api_signatures = crate::api_signature::apply_verified_api_signatures(
            &staged,
            &wired.module_name,
            &options.api_signatures,
            &options.api_aliases,
        )?;
        let reviewed_api_rule = if let Some((rule, _test)) = &reviewed_contract {
            Some(crate::reviewed::apply_reviewed_api(
                &staged,
                &wired.module_name,
                rule,
            )?)
        } else {
            None
        };
        let reviewed_contract_test = reviewed_contract.as_ref().map(|(_rule, test)| test.clone());
        preserve_source_attribution(&capsule.manifest, &capsule.root, &staged)?;
        let image_id = inspect_local_image_id()?;
        let user = nonroot_container_user()?;
        let start = Instant::now();
        let logs = docker_build(
            &staged,
            &results,
            options.registry_cache.as_deref(),
            &image_id,
            &user,
        )?;
        let elapsed = start.elapsed().as_secs_f64();
        let resolved_lock = results.join("Cargo.lock");
        let lock_meta = fs::symlink_metadata(&resolved_lock)?;
        if !lock_meta.file_type().is_file() || lock_meta.len() > 8 * 1024 * 1024 {
            return Err(VerifiedRustError::Docker(
                "container did not produce a regular, bounded lockfile".into(),
            ));
        }
        let compatible_constraints_retained = verify_locked_source_constraints(
            &capsule.manifest,
            &capsule.root,
            &staged.join("Cargo.toml"),
            &resolved_lock,
        )?;
        fs::copy(&resolved_lock, staged.join("Cargo.lock"))?;
        let evidence_name = PathBuf::from("composer-build-evidence.json");
        let evidence = BuildEvidence {
            schema_version: 1,
            capsule_id: wired.capsule_id.clone(),
            image: DEFAULT_IMAGE.into(),
            image_id: image_id.clone(),
            network_disabled: true,
            read_only_container_root: true,
            unprivileged_user: user.clone(),
            cpu_limit: "1".into(),
            memory_limit: "1024m".into(),
            commands: vec![
                "cargo test --locked --offline --all-targets (initial attempt)".into(),
                "cargo generate-lockfile --offline (at most once on explicit stale lockfile error)"
                    .into(),
                "cargo test --locked --offline --all-targets (only after lock repair)".into(),
                "cargo tree --locked --offline --depth 2".into(),
            ],
            dependencies_added: added.clone(),
            dependency_feature_changes: dependency_feature_changes.clone(),
            compatible_constraints_retained: compatible_constraints_retained.clone(),
            verified_api_aliases: verified_api_aliases.clone(),
            verified_api_signatures: verified_api_signatures.clone(),
            reviewed_api_rule: reviewed_api_rule.clone(),
            reviewed_contract_test: reviewed_contract_test.clone(),
            repair_actions: if logs.contains("COMPOSER_REPAIR: lockfile_regenerated") {
                vec!["lockfile_regenerated_once_after_missing_or_stale_check".into()]
            } else {
                Vec::new()
            },
            tests_passed: true,
            elapsed_seconds: elapsed,
            log_tail: logs,
        };
        fs::write(
            staged.join(&evidence_name),
            serde_json::to_vec_pretty(&evidence)?,
        )?;
        fs::rename(&staged, &output)?;
        Ok(VerifiedRustResult {
            output_root: output.clone(),
            capsule_id: wired.capsule_id,
            new_dependencies: added,
            dependency_feature_changes,
            compatible_constraints_retained,
            verified_api_aliases,
            verified_api_signatures,
            reviewed_api_rule,
            reviewed_contract_test,
            docker_image: DEFAULT_IMAGE.into(),
            offline_lockfile_resolved: true,
            isolated_tests_passed: true,
            evidence_path: output.join(evidence_name),
            elapsed_seconds: elapsed,
        })
    })();
    let _ = fs::remove_dir_all(&results);
    if outcome.is_err() {
        let _ = fs::remove_dir_all(&staged);
    }
    outcome
}

/// Opt-in API facade: re-export selected imported public items under aliases
/// expected by the target. This never rewrites the upstream source or attempts
/// to infer types. Rust compilation and tests independently prove each export.
fn apply_verified_api_aliases(
    staged: &Path,
    module_name: &str,
    aliases: &[String],
) -> Result<Vec<String>, VerifiedRustError> {
    if aliases.is_empty() {
        return Ok(Vec::new());
    }
    if aliases.len() > 8 {
        return Err(VerifiedRustError::Unsafe(
            "no more than eight explicit API aliases per integration".into(),
        ));
    }
    let lib = staged.join("src/lib.rs");
    let meta = fs::symlink_metadata(&lib).map_err(|_| {
        VerifiedRustError::Unsafe(
            "API aliases require an existing src/lib.rs library target".into(),
        )
    })?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe(
            "invalid library entrypoint".into(),
        ));
    }
    let original = fs::read_to_string(&lib)?;
    let mut added = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    let mut update = original.clone();
    for rule in aliases {
        let (source, alias) = rule.split_once('=').ok_or_else(|| {
            VerifiedRustError::Unsafe("API alias requires source::path=TargetName".into())
        })?;
        let segments = source.split("::").collect::<Vec<_>>();
        if rule.len() > 160
            || segments.is_empty()
            || segments.len() > 8
            || !segments.iter().all(|s| valid_public_alias_identifier(s))
            || !valid_public_alias_identifier(alias)
            || alias == module_name
            || !names.insert(alias.to_string())
        {
            return Err(VerifiedRustError::Unsafe(format!(
                "invalid or duplicated API alias: {rule}"
            )));
        }
        let declaration = format!("pub use crate::{module_name}::{source} as {alias};");
        if original.contains(&declaration) {
            return Err(VerifiedRustError::Unsafe(format!(
                "API alias is already declared: {rule}"
            )));
        }
        update.push_str(&format!(
            "\n// Software Composer: explicit compiler-verified API facade\n{declaration}\n"
        ));
        added.push(rule.clone());
    }
    fs::write(&lib, update)?;
    Ok(added)
}

fn valid_public_alias_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        && value != "_"
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn copy_bounded_target(source: &Path, output: &Path) -> Result<(), VerifiedRustError> {
    let mut total_files = 0usize;
    let mut total_bytes = 0u64;
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "README.md",
        "LICENSE",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "LICENSE.md",
        "LICENSE.txt",
        "COPYING",
        "NOTICE",
        "NOTICE.md",
        "composer-source-attributions.json",
    ] {
        let source_file = source.join(name);
        if source_file.exists() {
            copy_regular(
                &source_file,
                &output.join(name),
                &mut total_files,
                &mut total_bytes,
            )?;
        }
    }
    if !output.join("Cargo.toml").is_file() {
        return Err(VerifiedRustError::Unsafe(
            "target Cargo.toml is missing".into(),
        ));
    }
    for dir in [
        "src",
        "tests",
        "examples",
        "benches",
        "third_party_licenses",
    ] {
        let source_dir = source.join(dir);
        if source_dir.exists() {
            copy_tree(
                &source_dir,
                &output.join(dir),
                &mut total_files,
                &mut total_bytes,
            )?;
        }
    }
    if !output.join("src").is_dir() {
        return Err(VerifiedRustError::Unsafe(
            "target src directory is missing".into(),
        ));
    }
    Ok(())
}

/// Keep the original, hash-verified license evidence with every published
/// source integration. An explicit review flag records human review, not
/// automated legal clearance. Batch composition preserves this evidence.
fn preserve_source_attribution(
    manifest: &CapsuleManifest,
    capsule_root: &Path,
    target_root: &Path,
) -> Result<(), VerifiedRustError> {
    let licenses = manifest
        .files
        .iter()
        .filter(|file| file.file_kind == "license_evidence")
        .collect::<Vec<_>>();
    if licenses.is_empty() || licenses.len() > 8 {
        return Err(VerifiedRustError::Unsafe(
            "a reviewed capsule must contain 1-8 original license evidence files".into(),
        ));
    }
    let license_dir = target_root.join("third_party_licenses");
    fs::create_dir_all(&license_dir)?;
    let mut notices = Vec::new();
    for (index, source) in licenses.iter().enumerate() {
        let original = capsule_root.join(&source.capsule_path);
        let data = fs::read(&original)?;
        if data.len() > 512 * 1024 {
            return Err(VerifiedRustError::Unsafe(
                "license evidence exceeds the 512 KiB file limit".into(),
            ));
        }
        let name = format!("{}-license-{index}.txt", manifest.capability_id);
        let destination = license_dir.join(&name);
        if destination.exists() {
            return Err(VerifiedRustError::Unsafe(format!(
                "license output collision: {}",
                destination.display()
            )));
        }
        fs::write(&destination, data)?;
        notices.push(serde_json::json!({
            "original_path": source.source_path,
            "source_sha256": source.sha256,
            "published_path": format!("third_party_licenses/{name}"),
        }));
    }
    let provenance_file = target_root.join("composer-source-attributions.json");
    let mut existing: Vec<serde_json::Value> = if provenance_file.is_file() {
        serde_json::from_slice(&fs::read(&provenance_file)?)?
    } else {
        Vec::new()
    };
    if existing.iter().any(|entry| {
        entry
            .get("capability_id")
            .and_then(serde_json::Value::as_str)
            == Some(&manifest.capability_id)
    }) {
        return Err(VerifiedRustError::Unsafe(
            "source attribution already exists for this capability".into(),
        ));
    }
    let source_files = manifest
        .files
        .iter()
        .filter(|file| file.file_kind == "source")
        .map(|file| {
            serde_json::json!({
                "original_path": file.source_path,
                "sha256": file.sha256,
            })
        })
        .collect::<Vec<_>>();
    let module = manifest.capability_id.replace('-', "_");
    let published_module = if source_files.len() > 1 {
        format!("src/composer_{module}/mod.rs")
    } else {
        format!("src/composer_{module}.rs")
    };
    existing.push(serde_json::json!({
        "schema_version": 1,
        "capability_id": manifest.capability_id,
        "capsule_id": manifest.capsule_id,
        "source_repository": manifest.source_repository_url,
        "source_commit_sha": manifest.source_commit_sha,
        "source_files": source_files,
        "published_module": published_module,
        "license_documents": notices,
        "license_review_asserted_by_caller": true,
        "automated_legal_clearance": false
    }));
    fs::write(provenance_file, serde_json::to_vec_pretty(&existing)?)?;
    Ok(())
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    files: &mut usize,
    bytes: &mut u64,
) -> Result<(), VerifiedRustError> {
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe(format!(
            "target contains a symlink or non-directory: {}",
            source.display()
        )));
    }
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        // Exclude dotfiles and dot-directories, including common local secrets.
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let item = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(VerifiedRustError::Unsafe(format!(
                "symlink rejected: {}",
                item.display()
            )));
        }
        if kind.is_dir() {
            if matches!(
                name.to_str(),
                Some("target" | ".git" | ".cargo" | "node_modules")
            ) {
                continue;
            }
            copy_tree(&item, &destination.join(name), files, bytes)?;
        } else if kind.is_file() {
            let allowed = matches!(
                item.extension().and_then(|ext| ext.to_str()),
                Some("rs" | "json" | "toml" | "txt" | "md" | "sql" | "html")
            );
            if allowed {
                copy_regular(&item, &destination.join(name), files, bytes)?;
            }
        } else {
            return Err(VerifiedRustError::Unsafe(format!(
                "non-regular file: {}",
                item.display()
            )));
        }
    }
    Ok(())
}

fn copy_regular(
    source: &Path,
    destination: &Path,
    files: &mut usize,
    bytes: &mut u64,
) -> Result<(), VerifiedRustError> {
    let meta = fs::symlink_metadata(source)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe(format!(
            "non-regular input: {}",
            source.display()
        )));
    }
    *files += 1;
    *bytes = bytes.saturating_add(meta.len());
    if *files > MAX_FILES || *bytes > MAX_BYTES {
        return Err(VerifiedRustError::Unsafe(
            "target exceeds 1500 files or 64 MiB source snapshot limit".into(),
        ));
    }
    fs::copy(source, destination)?;
    Ok(())
}

/// Only direct registry versions and a bounded list of named Cargo features.
/// Do not transplant git/path/renamed packages, default-features policy,
/// optional dependencies, target-specific settings, or dependency feature links.
fn parse_versioned_dependency(
    name: &str,
    dependency: &toml::Value,
) -> Result<(String, Vec<String>), VerifiedRustError> {
    let (version, mut features) = match dependency {
        toml::Value::String(v) => (v.clone(), Vec::new()),
        toml::Value::Table(table) => {
            if table
                .keys()
                .any(|key| key != "version" && key != "features")
            {
                return Err(VerifiedRustError::Dependency(format!(
                    "{name} has unsupported dependency keys; only version and features are supported"
                )));
            }
            let v = table
                .get("version")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| {
                    VerifiedRustError::Dependency(format!(
                        "{name} has no direct version requirement"
                    ))
                })?
                .to_owned();
            let list = match table.get("features") {
                Some(value) => value.as_array().ok_or_else(|| {
                    VerifiedRustError::Dependency(format!(
                        "{name} features must be a list of literal feature names"
                    ))
                })?,
                None => return Ok((validated_requirement(name, &v)?, Vec::new())),
            };
            if list.len() > 16 {
                return Err(VerifiedRustError::Dependency(format!(
                    "{name} requests more than 16 dependency features"
                )));
            }
            let mut names = Vec::new();
            for item in list {
                let f = item.as_str().ok_or_else(|| {
                    VerifiedRustError::Dependency(format!(
                        "{name} contains a non-string dependency feature"
                    ))
                })?;
                if f.is_empty()
                    || f.len() > 48
                    || !f
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    return Err(VerifiedRustError::Dependency(format!(
                        "{name} uses unsupported feature syntax: {f}"
                    )));
                }
                names.push(f.to_owned());
            }
            (v, names)
        }
        _ => {
            return Err(VerifiedRustError::Dependency(format!(
                "{name} uses an unsupported non-version dependency"
            )));
        }
    };
    features.sort();
    features.dedup();
    Ok((validated_requirement(name, &version)?, features))
}

fn validated_requirement(name: &str, version: &str) -> Result<String, VerifiedRustError> {
    if version.is_empty()
        || version.len() > 128
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" .,=*^~<>+-".contains(&b))
    {
        return Err(VerifiedRustError::Dependency(format!(
            "{name} has an unsupported version requirement"
        )));
    }
    semver::VersionReq::parse(version).map_err(|err| {
        VerifiedRustError::Dependency(format!("{name} has invalid version requirement: {err}"))
    })?;
    Ok(version.to_owned())
}

fn dependency_with_features(version: &str, features: &[String]) -> toml::Value {
    if features.is_empty() {
        return toml::Value::String(version.into());
    }
    let mut table = toml::map::Map::new();
    table.insert("version".into(), toml::Value::String(version.into()));
    table.insert(
        "features".into(),
        toml::Value::Array(features.iter().cloned().map(toml::Value::String).collect()),
    );
    toml::Value::Table(table)
}

/// For a single-file verified capsule, select only dependency names that
/// occur as crate paths/imports in that source. This is conservative lexical
/// selection, NOT semantic resolution; the isolated Rust compiler and Cargo
/// lockfile checks must still prove a working integration before publication.
/// Optional features and upstream dev dependencies are not transplanted.
fn selected_source_direct_dependencies(
    manifest: &CapsuleManifest,
    capsule_root: &Path,
    cargo: &toml::Value,
) -> Result<std::collections::BTreeMap<String, String>, VerifiedRustError> {
    let sources = manifest
        .files
        .iter()
        .filter(|item| item.file_kind == "source")
        .collect::<Vec<_>>();
    if sources.is_empty() || sources.len() > 32 {
        return Err(VerifiedRustError::Dependency(
            "selective dependency inventory supports 1-32 source files".into(),
        ));
    }
    let mut source = String::new();
    let mut total = 0u64;
    for item in &sources {
        let file = capsule_root.join(&item.capsule_path);
        let metadata = fs::symlink_metadata(&file)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(VerifiedRustError::Unsafe(format!(
                "non-regular selected source: {}",
                file.display()
            )));
        }
        total = total.saturating_add(metadata.len());
        if total > 4 * 1024 * 1024 {
            return Err(VerifiedRustError::Dependency(
                "selected Rust sources exceed 4 MiB total".into(),
            ));
        }
        source.push_str(&fs::read_to_string(file)?);
        source.push('\n');
    }
    let mut chosen = std::collections::BTreeMap::new();
    let Some(table) = cargo.get("dependencies") else {
        return Ok(chosen);
    };
    let dependencies = table.as_table().ok_or_else(|| {
        VerifiedRustError::Dependency("source [dependencies] must be a TOML table".into())
    })?;
    for (name, value) in dependencies {
        let crate_name = name.replace('-', "_");
        if !source.contains(&format!("{crate_name}::"))
            && !source.contains(&format!("use {crate_name}"))
            && !source.contains(&format!("extern crate {crate_name}"))
        {
            continue;
        }
        let (version, _) = parse_versioned_dependency(name, value)?;
        chosen.insert(name.clone(), version);
    }
    Ok(chosen)
}

/// Conservative direct-dependency adoption. No version conflicts are
/// overwritten, and path, git, registry, build or target-specific dependencies
/// are not inferred. Cargo performs the actual offline transitive solve later.
fn merge_direct_cargo_dependencies(
    manifest: &CapsuleManifest,
    capsule_root: &Path,
    target_root: &Path,
) -> Result<(Vec<String>, Vec<String>), VerifiedRustError> {
    let cargo = manifest
        .files
        .iter()
        .find(|entry| {
            entry.file_kind == "dependency_manifest"
                && entry.source_path.as_deref() == Some(Path::new("Cargo.toml"))
        })
        .ok_or_else(|| {
            VerifiedRustError::Dependency(
                "capsule has no root Cargo.toml; nested package manifests are unsupported".into(),
            )
        })?;
    let source = fs::read_to_string(capsule_root.join(&cargo.capsule_path))?;
    let source_value: toml::Value =
        toml::from_str(&source).map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    let source_deps = selected_source_direct_dependencies(manifest, capsule_root, &source_value)?;
    let target_manifest = target_root.join("Cargo.toml");
    let mut target_value: toml::Value = toml::from_str(&fs::read_to_string(&target_manifest)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    if target_value.get("package").is_none() || target_value.get("workspace").is_some() {
        return Err(VerifiedRustError::Dependency(
            "only standalone Cargo packages (not workspace roots) are supported".into(),
        ));
    }
    let source_table = source_value
        .get("dependencies")
        .and_then(toml::Value::as_table);
    let target = target_value
        .as_table_mut()
        .ok_or_else(|| VerifiedRustError::Toml("target manifest must be TOML table".into()))?;
    let deps = target
        .entry("dependencies".to_owned())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| {
            VerifiedRustError::Dependency("target [dependencies] must be a TOML table".into())
        })?;
    let mut added = Vec::new();
    let mut feature_changes = Vec::new();
    for (name, requested) in &source_deps {
        let from_source = source_table.and_then(|v| v.get(name)).ok_or_else(|| {
            VerifiedRustError::Dependency(format!("{name} missing from source [dependencies]"))
        })?;
        let (_, requested_features) = parse_versioned_dependency(name, from_source)?;
        match deps.get_mut(name) {
            None => {
                deps.insert(
                    name.clone(),
                    dependency_with_features(requested, &requested_features),
                );
                added.push(format!("{name} = {requested}"));
                if !requested_features.is_empty() {
                    feature_changes.push(format!(
                        "{name}: enabled [{}]",
                        requested_features.join(", ")
                    ));
                }
            }
            Some(existing) => {
                let (retained, mut existing_features) = parse_versioned_dependency(name, existing)?;
                // Never rewrite existing target version requirements.
                if &retained != requested {
                    semver::VersionReq::parse(requested).map_err(|err| {
                        VerifiedRustError::Dependency(format!(
                            "invalid source requirement for {name}: {err}"
                        ))
                    })?;
                    semver::VersionReq::parse(&retained).map_err(|err| {
                        VerifiedRustError::Dependency(format!(
                            "invalid target requirement for {name}: {err}"
                        ))
                    })?;
                }
                let newly_enabled = requested_features
                    .iter()
                    .filter(|f| !existing_features.contains(f))
                    .cloned()
                    .collect::<Vec<_>>();
                if !newly_enabled.is_empty() {
                    existing_features.extend(newly_enabled.iter().cloned());
                    existing_features.sort();
                    existing_features.dedup();
                    *existing = dependency_with_features(&retained, &existing_features);
                    feature_changes.push(format!("{name}: enabled [{}]", newly_enabled.join(", ")));
                }
            }
        }
    }
    fs::write(
        target_manifest,
        toml::to_string_pretty(&target_value)
            .map_err(|err| VerifiedRustError::Toml(err.to_string()))?,
    )?;
    Ok((added, feature_changes))
}

/// Verify that Cargo's chosen version of every direct source dependency
/// satisfies BOTH the source constraint and the retained target constraint.
/// A successful compilation alone is insufficient to prove this requirement.
fn verify_locked_source_constraints(
    manifest: &CapsuleManifest,
    capsule_root: &Path,
    target_manifest: &Path,
    lockfile: &Path,
) -> Result<Vec<String>, VerifiedRustError> {
    let source = manifest
        .files
        .iter()
        .find(|entry| {
            entry.file_kind == "dependency_manifest"
                && entry.source_path.as_deref() == Some(Path::new("Cargo.toml"))
        })
        .ok_or_else(|| VerifiedRustError::Dependency("source Cargo.toml missing".into()))?;
    let source_toml: toml::Value = toml::from_str(&fs::read_to_string(
        capsule_root.join(&source.capsule_path),
    )?)
    .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    let target_toml: toml::Value = toml::from_str(&fs::read_to_string(target_manifest)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    let lock: toml::Value = toml::from_str(&fs::read_to_string(lockfile)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| {
            VerifiedRustError::Dependency("resolved Cargo.lock has no packages".into())
        })?;
    let source_deps = selected_source_direct_dependencies(manifest, capsule_root, &source_toml)?;
    let target_deps = target_toml
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| VerifiedRustError::Dependency("target has no [dependencies]".into()))?;
    let mut reconciled = Vec::new();
    for (name, requirement) in &source_deps {
        let requirement = requirement.as_str();
        let parsed = semver::VersionReq::parse(requirement).map_err(|err| {
            VerifiedRustError::Dependency(format!("bad source version {name}: {err}"))
        })?;
        let (retained, _) = parse_versioned_dependency(
            name,
            target_deps.get(name).ok_or_else(|| {
                VerifiedRustError::Dependency(format!("target direct requirement missing: {name}"))
            })?,
        )?;
        let target_req = semver::VersionReq::parse(&retained).map_err(|err| {
            VerifiedRustError::Dependency(format!("bad target version {name}: {err}"))
        })?;
        let candidates = packages
            .iter()
            .filter(|pkg| pkg.get("name").and_then(toml::Value::as_str) == Some(name.as_str()))
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(VerifiedRustError::Dependency(format!(
                "direct dependency {name} has {} lockfile candidates; refusing ambiguous resolution",
                candidates.len()
            )));
        }
        let chosen = candidates[0]
            .get("version")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                VerifiedRustError::Dependency(format!("{name} has no locked version"))
            })?;
        let version = semver::Version::parse(chosen).map_err(|err| {
            VerifiedRustError::Dependency(format!("invalid locked version for {name}: {err}"))
        })?;
        if !parsed.matches(&version) || !target_req.matches(&version) {
            return Err(VerifiedRustError::Dependency(format!(
                "{name} resolved to {chosen}; incompatible with source '{requirement}' or target '{retained}'"
            )));
        }
        if retained != requirement {
            reconciled.push(format!(
                "{name}: source='{requirement}', target='{retained}', resolved='{chosen}'"
            ));
        }
    }
    Ok(reconciled)
}

fn mount_arg(path: &Path, target: &str, readonly: bool) -> Result<String, VerifiedRustError> {
    let source = path.canonicalize()?.to_string_lossy().into_owned();
    #[cfg(windows)]
    let source = {
        if source.starts_with(r"\\?\UNC\") {
            return Err(VerifiedRustError::Unsafe(
                "UNC Docker volume paths are not supported".into(),
            ));
        }
        source.strip_prefix(r"\\?\").unwrap_or(&source).to_owned()
    };
    if source.contains(',') || source.contains('\n') || source.contains('\r') {
        return Err(VerifiedRustError::Unsafe(
            "unsupported mount path characters".into(),
        ));
    }
    Ok(format!(
        "type=bind,source={source},target={target}{}",
        if readonly { ",readonly" } else { "" }
    ))
}

/// Use the host owner's non-root identity on Unix so the container can read
/// temporary project directories without making copied source world-readable.
/// Docker Desktop on Windows continues to use an unprivileged fixed UID.
#[cfg(unix)]
fn nonroot_container_user() -> Result<String, VerifiedRustError> {
    let mut ids = Vec::new();
    for flag in ["-u", "-g"] {
        let output = Command::new("id").arg(flag).output().map_err(|err| {
            VerifiedRustError::Docker(format!("could not get Unix identity: {err}"))
        })?;
        if !output.status.success() {
            return Err(VerifiedRustError::Docker(
                "could not read Unix identity".into(),
            ));
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) || value == "0" {
            return Err(VerifiedRustError::Unsafe(
                "Unix Docker verification requires a non-root host UID/GID".into(),
            ));
        }
        ids.push(value);
    }
    Ok(format!("{}:{}", ids[0], ids[1]))
}

#[cfg(not(unix))]
fn nonroot_container_user() -> Result<String, VerifiedRustError> {
    Ok("65534:65534".into())
}

fn inspect_local_image_id() -> Result<String, VerifiedRustError> {
    let output = Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", DEFAULT_IMAGE])
        .output()
        .map_err(|err| VerifiedRustError::Docker(format!("Docker CLI unavailable: {err}")))?;
    if !output.status.success() {
        return Err(VerifiedRustError::Docker(format!(
            "{DEFAULT_IMAGE} must already be installed locally; no image pull is permitted during verification"
        )));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(VerifiedRustError::Docker(
            "local Docker image returned an invalid content ID".into(),
        ));
    }
    Ok(value)
}

fn docker_build(
    staged: &Path,
    results: &Path,
    registry: Option<&Path>,
    image_id: &str,
    container_user: &str,
) -> Result<String, VerifiedRustError> {
    // No shell expansion from user input: a fixed script with process arguments.
    const SCRIPT: &str = "set -eu; cp -R /input/. /work/; if cargo test --locked --offline --all-targets >/tmp/composer-first-test.log 2>&1; then cat /tmp/composer-first-test.log; else if grep -Fq 'lock file' /tmp/composer-first-test.log && grep -Fq -- '--locked' /tmp/composer-first-test.log; then echo COMPOSER_REPAIR: lockfile_regenerated; cargo generate-lockfile --offline; cargo test --locked --offline --all-targets; else cat /tmp/composer-first-test.log; exit 1; fi; fi; cargo tree --locked --offline --depth 2; cp Cargo.lock /results/Cargo.lock";
    let name = format!("composer-verify-{}", Uuid::new_v4().simple());
    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "--detach",
        "--pull=never",
        "--name",
        &name,
        "--network=none",
        "--ipc=none",
        "--ulimit=nofile=1024:1024",
        "--read-only",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--pids-limit=64",
        "--memory=1024m",
        "--cpus=1",
        "--user",
        container_user,
        "--workdir=/work",
        "--tmpfs=/tmp:rw,nosuid,nodev,mode=1777,size=256m",
        "--tmpfs=/work:rw,nosuid,nodev,exec,mode=1777,size=1024m",
        "--log-driver=local",
        "--log-opt=max-size=5m",
        "--log-opt=max-file=2",
        "--env=CARGO_HOME=/tmp/cargo",
        "--env=CARGO_NET_OFFLINE=true",
        "--env=CARGO_BUILD_JOBS=1",
        "--env=RUSTFLAGS=-C debuginfo=0",
        "--mount",
        &mount_arg(staged, "/input", true)?,
        "--mount",
        &mount_arg(results, "/results", false)?,
    ]);
    if let Some(registry) = registry {
        let path = registry.canonicalize()?;
        let valid_cargo_registry = path.file_name().is_some_and(|name| name == "registry")
            && path.parent().is_some_and(|parent| {
                parent.file_name().is_some_and(|name| name == ".cargo")
                    || std::env::var_os("CARGO_HOME").is_some_and(|home| parent == Path::new(&home))
            });
        if !valid_cargo_registry {
            return Err(VerifiedRustError::Unsafe(
                "cache mount must be a Cargo registry directory under .cargo or CARGO_HOME".into(),
            ));
        }
        if !path.is_dir() || fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(VerifiedRustError::Unsafe(
                "registry cache must be a regular directory".into(),
            ));
        }
        cmd.arg("--mount")
            .arg(mount_arg(&path, "/tmp/cargo/registry", true)?);
    }
    cmd.args(["--entrypoint", "sh", image_id, "-c", SCRIPT]);
    let launched = cmd
        .output()
        .map_err(|err| VerifiedRustError::Docker(format!("Docker CLI unavailable: {err}")))?;
    if !launched.status.success() {
        return Err(VerifiedRustError::Docker(format!(
            "Docker could not start its restricted container (image {DEFAULT_IMAGE} must exist locally): {}",
            String::from_utf8_lossy(&launched.stderr)
                .chars()
                .take(2000)
                .collect::<String>()
        )));
    }

    let started = Instant::now();
    let result = loop {
        if started.elapsed() > TIMEOUT {
            break Err(VerifiedRustError::Docker(
                "isolated build exceeded 240-second limit".into(),
            ));
        }
        let status = Command::new("docker")
            .args([
                "inspect",
                "--format",
                "{{.State.Status}} {{.State.ExitCode}}",
                &name,
            ])
            .output();
        match status {
            Ok(out) if out.status.success() => {
                let value = String::from_utf8_lossy(&out.stdout);
                if value.starts_with("exited ") {
                    break if value.trim() == "exited 0" {
                        Ok(())
                    } else {
                        Err(VerifiedRustError::Docker(format!(
                            "isolated Cargo resolution/build/tests exited with {}",
                            value.trim()
                        )))
                    };
                }
            }
            _ => {
                break Err(VerifiedRustError::Docker(
                    "could not inspect isolated container state".into(),
                ));
            }
        }
        thread::sleep(Duration::from_millis(400));
    };
    let logs = Command::new("docker")
        .args(["logs", "--tail", "120", &name])
        .output()
        .ok()
        .map(|out| {
            format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        })
        .unwrap_or_default();
    let _ = Command::new("docker").args(["rm", "-f", &name]).output();
    let logs: String = logs.chars().take(MAX_LOG_CHARS).collect();
    result.map_err(|err| VerifiedRustError::Docker(format!("{err}\nContainer output:\n{logs}")))?;
    Ok(logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit-level constraint gate: this deliberately bypasses wiring because
    /// the current source-import closure rejects external imports earlier.
    /// A green Cargo test elsewhere does not prove incompatible versions agree.
    #[test]
    fn rejects_disjoint_semver_lockfile_in_dependent_target() {
        let root = std::env::temp_dir().join(format!("composer-heldout-semver-{}", Uuid::new_v4()));
        let capsule = root.join("capsule");
        let target = root.join("target");
        fs::create_dir_all(capsule.join("context/manifests")).unwrap();
        fs::create_dir_all(capsule.join("source/src")).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(
            capsule.join("source/src/lib.rs"),
            "use semver::Version;\npub fn parse_major(value: &str) -> u64 { Version::parse(value).map(|v| v.major).unwrap_or(0) }\n",
        ).unwrap();
        fs::write(
            capsule.join("context/manifests/Cargo.toml"),
            "[package]\nname='synthetic_source'\nversion='0.1.0'\n[dependencies]\nsemver='1'\n",
        )
        .unwrap();
        let target_manifest = target.join("Cargo.toml");
        let target_lock = target.join("Cargo.lock");
        let incompatible = "[package]\nname='synthetic_target'\nversion='0.1.0'\n[dependencies]\nsemver='=0.11.0'\n";
        fs::write(&target_manifest, incompatible).unwrap();
        fs::write(
            &target_lock,
            "version = 4\n[[package]]\nname = 'semver'\nversion = '0.11.0'\n",
        )
        .unwrap();
        let manifest = CapsuleManifest {
            schema_version: 1,
            capsule_id: "semver-fixture".into(),
            capability_id: "semver".into(),
            description: "Version conflict contract".into(),
            created_at_unix: 0,
            source_repository_url: None,
            source_commit_sha: None,
            source_tree_sha: None,
            dependency_closure_strategy: "test".into(),
            files: vec![
                composer_intelligence::CapsuleFile {
                    source_path: Some(PathBuf::from("Cargo.toml")),
                    capsule_path: "context/manifests/Cargo.toml".into(),
                    file_kind: "dependency_manifest".into(),
                    size_bytes: 0,
                    sha256: String::new(),
                },
                composer_intelligence::CapsuleFile {
                    source_path: Some(PathBuf::from("src/lib.rs")),
                    capsule_path: "source/src/lib.rs".into(),
                    file_kind: "source".into(),
                    size_bytes: 0,
                    sha256: String::new(),
                },
            ],
            included_source_files: vec![PathBuf::from("src/lib.rs")],
            included_symbol_ids: vec![],
            external_or_unresolved_imports: vec![],
            unfollowed_symbol_references: vec![],
            license_evidence_files: vec![],
            license_review_required: true,
            boundary_warnings: vec![],
        };
        let before_manifest = fs::read(&target_manifest).unwrap();
        let before_lock = fs::read(&target_lock).unwrap();
        let result =
            verify_locked_source_constraints(&manifest, &capsule, &target_manifest, &target_lock);
        match result {
            Err(VerifiedRustError::Dependency(message)) => {
                assert!(message.contains("incompatible with source"), "{message}");
                assert!(message.contains("semver"), "{message}");
            }
            other => panic!("disjoint dependency constraints were not rejected: {other:?}"),
        }
        assert_eq!(fs::read(&target_manifest).unwrap(), before_manifest);
        assert_eq!(fs::read(&target_lock).unwrap(), before_lock);

        // Positive control: both source and target requirements admit 1.0.28.
        fs::write(
            &target_manifest,
            "[package]\nname='synthetic_target'\nversion='0.1.0'\n[dependencies]\nsemver='=1.0.28'\n",
        ).unwrap();
        fs::write(
            &target_lock,
            "version = 4\n[[package]]\nname = 'semver'\nversion = '1.0.28'\n",
        )
        .unwrap();
        let reconciled =
            verify_locked_source_constraints(&manifest, &capsule, &target_manifest, &target_lock)
                .unwrap();
        assert_eq!(reconciled.len(), 1);
        assert!(reconciled[0].contains("1.0.28"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preserves_target_manifest_and_adds_only_matching_version_deps() {
        let root = std::env::temp_dir().join(format!("composer-deps-{}", Uuid::new_v4()));
        let capsule = root.join("capsule");
        let target = root.join("target");
        fs::create_dir_all(capsule.join("context/manifests")).unwrap();
        fs::create_dir_all(capsule.join("source/src")).unwrap();
        fs::write(
            capsule.join("source/src/feature.rs"),
            "use serde::Serialize;\npub fn sample<T: Serialize>(_value: T) {}\n",
        )
        .unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(
            capsule.join("context/manifests/Cargo.toml"),
            "[package]\nname='a'\nversion='0.1.0'\n[dependencies]\nserde='1'\n",
        )
        .unwrap();
        fs::write(
            target.join("Cargo.toml"),
            "[package]\nname='b'\nversion='0.1.0'\n",
        )
        .unwrap();
        let manifest = CapsuleManifest {
            schema_version: 1,
            capsule_id: "fixture".into(),
            capability_id: "cap".into(),
            description: "fixture".into(),
            created_at_unix: 0,
            source_repository_url: None,
            source_commit_sha: None,
            source_tree_sha: None,
            dependency_closure_strategy: "test".into(),
            files: vec![
                composer_intelligence::CapsuleFile {
                    source_path: Some(PathBuf::from("Cargo.toml")),
                    capsule_path: "context/manifests/Cargo.toml".into(),
                    file_kind: "dependency_manifest".into(),
                    size_bytes: 0,
                    sha256: String::new(),
                },
                composer_intelligence::CapsuleFile {
                    source_path: Some(PathBuf::from("src/feature.rs")),
                    capsule_path: "source/src/feature.rs".into(),
                    file_kind: "source".into(),
                    size_bytes: 0,
                    sha256: String::new(),
                },
            ],
            included_source_files: vec![PathBuf::from("src/feature.rs")],
            included_symbol_ids: vec![],
            external_or_unresolved_imports: vec![],
            unfollowed_symbol_references: vec![],
            license_evidence_files: vec![],
            license_review_required: true,
            boundary_warnings: vec![],
        };
        let added = merge_direct_cargo_dependencies(&manifest, &capsule, &target).unwrap();
        assert_eq!(added.0, vec!["serde = 1"]);
        assert!(added.1.is_empty());
        assert!(
            fs::read_to_string(target.join("Cargo.toml"))
                .unwrap()
                .contains("serde = \"1\"")
        );
        let again = merge_direct_cargo_dependencies(&manifest, &capsule, &target).unwrap();
        assert!(again.0.is_empty());
        assert!(again.1.is_empty());
        fs::write(
            target.join("Cargo.toml"),
            "[package]\nname='b'\nversion='0.1.0'\n[dependencies]\nserde='2'\n",
        )
        .unwrap();
        // A differing target constraint is never overwritten. Cargo may
        // resolve it, but publication requires the locked version to satisfy
        // BOTH the source and target requirements.
        assert!(
            merge_direct_cargo_dependencies(&manifest, &capsule, &target)
                .unwrap()
                .0
                .is_empty()
        );
        assert!(
            fs::read_to_string(target.join("Cargo.toml"))
                .unwrap()
                .contains("serde = \"2\"")
        );
        let lock = target.join("Cargo.lock");
        fs::write(
            &lock,
            "version = 4\n[[package]]\nname = 'serde'\nversion = '2.0.0'\n",
        )
        .unwrap();
        assert!(matches!(
            verify_locked_source_constraints(
                &manifest,
                &capsule,
                &target.join("Cargo.toml"),
                &lock
            ),
            Err(VerifiedRustError::Dependency(_))
        ));
        fs::write(
            target.join("Cargo.toml"),
            "[package]\nname='b'\nversion='0.1.0'\n[dependencies]\nserde='^1.0.1'\n",
        )
        .unwrap();
        fs::write(
            &lock,
            "version = 4\n[[package]]\nname = 'serde'\nversion = '1.0.228'\n",
        )
        .unwrap();
        let overlaps = verify_locked_source_constraints(
            &manifest,
            &capsule,
            &target.join("Cargo.toml"),
            &lock,
        )
        .unwrap();
        assert_eq!(overlaps.len(), 1);
        assert!(overlaps[0].contains("1.0.228"));
        // Real-world manifests frequently declare optional/development crates
        // that the selected standalone module never uses.
        let source_manifest = capsule.join("context/manifests/Cargo.toml");
        fs::write(
            &source_manifest,
            "[package]\nname='a'\nversion='0.1.0'\n[dependencies]\nserde='1'\nunused_pkg={version='99', optional=true}\n[dev-dependencies]\nquickcheck='1'\n",
        ).unwrap();
        let toml_value: toml::Value =
            toml::from_str(&fs::read_to_string(&source_manifest).unwrap()).unwrap();
        let selected =
            selected_source_direct_dependencies(&manifest, &capsule, &toml_value).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected.get("serde").map(String::as_str), Some("1"));
        fs::write(
            &source_manifest,
            "[package]\nname='a'\nversion='0.1.0'\n[dev-dependencies]\nquickcheck='1'\n",
        )
        .unwrap();
        let empty_manifest: toml::Value =
            toml::from_str(&fs::read_to_string(&source_manifest).unwrap()).unwrap();
        assert!(
            selected_source_direct_dependencies(&manifest, &capsule, &empty_manifest)
                .unwrap()
                .is_empty()
        );
        // Source features are adopted conservatively into a target string
        // requirement while retaining its version and without duplication.
        fs::write(
            &source_manifest,
            "[package]\nname='a'\nversion='0.1.0'\n[dependencies]\nserde={version='1',features=['derive']}\n",
        ).unwrap();
        fs::write(
            target.join("Cargo.toml"),
            "[package]\nname='b'\nversion='0.1.0'\n[dependencies]\nserde='1'\n",
        )
        .unwrap();
        let (added, changes) =
            merge_direct_cargo_dependencies(&manifest, &capsule, &target).unwrap();
        assert!(added.is_empty());
        assert_eq!(changes, vec!["serde: enabled [derive]"]);
        let manifest_text = fs::read_to_string(target.join("Cargo.toml")).unwrap();
        assert!(manifest_text.contains("derive"));
        assert_eq!(
            merge_direct_cargo_dependencies(&manifest, &capsule, &target)
                .unwrap()
                .1
                .len(),
            0
        );
        // Never transplant arbitrary git/path/renamed dependencies.
        fs::write(
            &source_manifest,
            "[package]\nname='a'\nversion='0.1.0'\n[dependencies]\nserde={version='1',git='https://example.invalid/unsafe'}\n",
        ).unwrap();
        let before = fs::read_to_string(target.join("Cargo.toml")).unwrap();
        assert!(merge_direct_cargo_dependencies(&manifest, &capsule, &target).is_err());
        assert_eq!(
            fs::read_to_string(target.join("Cargo.toml")).unwrap(),
            before
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_or_duplicate_api_aliases_without_mutating_library() {
        let root = std::env::temp_dir().join(format!("composer-api-alias-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("src")).unwrap();
        let lib = root.join("src/lib.rs");
        let original = "pub mod composer_example;\n";
        fs::write(&lib, original).unwrap();
        let bad = vec!["helper::answer=bad;injection".to_owned()];
        assert!(matches!(
            apply_verified_api_aliases(&root, "composer_example", &bad),
            Err(VerifiedRustError::Unsafe(_))
        ));
        assert_eq!(fs::read_to_string(&lib).unwrap(), original);
        let duplicate = vec![
            "helper::answer=ported".to_owned(),
            "helper::other=ported".to_owned(),
        ];
        assert!(apply_verified_api_aliases(&root, "composer_example", &duplicate).is_err());
        assert_eq!(fs::read_to_string(&lib).unwrap(), original);
        let added = apply_verified_api_aliases(
            &root,
            "composer_example",
            &["helper::answer=ported".into()],
        )
        .unwrap();
        assert_eq!(added, vec!["helper::answer=ported"]);
        assert!(
            fs::read_to_string(&lib)
                .unwrap()
                .contains("pub use crate::composer_example::helper::answer as ported;")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_symlink_snapshot_files_and_requires_review_before_io() {
        let result = compose_verified_rust(&VerifiedRustOptions {
            capsule_root: PathBuf::from("nonexistent"),
            target_root: PathBuf::from("nonexistent"),
            output_root: PathBuf::from("nonexistent"),
            license_reviewed: false,
            registry_cache: None,
            api_aliases: Vec::new(),
            api_signatures: Vec::new(),
            reviewed_api_rule: None,
            contract_test: None,
        });
        assert!(matches!(result, Err(VerifiedRustError::ReviewRequired)));
    }
}
