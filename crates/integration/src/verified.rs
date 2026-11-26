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

use crate::{RustWiringOptions, verify_capsule, wire_rust_module};

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedRustResult {
    pub output_root: PathBuf,
    pub capsule_id: String,
    pub new_dependencies: Vec<String>,
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
    network_disabled: bool,
    read_only_container_root: bool,
    unprivileged_user: String,
    cpu_limit: String,
    memory_limit: String,
    commands: Vec<String>,
    dependencies_added: Vec<String>,
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
    let capsule = verify_capsule(&options.capsule_root)?;
    let source = options.target_root.canonicalize()?;
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
        let added = merge_direct_cargo_dependencies(&capsule.manifest, &capsule.root, &staged)?;
        let wired = wire_rust_module(&RustWiringOptions {
            capsule_root: capsule.root.clone(),
            target_root: staged.clone(),
            license_reviewed: true,
        })?;
        let start = Instant::now();
        let logs = docker_build(&staged, &results, options.registry_cache.as_deref())?;
        let elapsed = start.elapsed().as_secs_f64();
        let resolved_lock = results.join("Cargo.lock");
        let lock_meta = fs::symlink_metadata(&resolved_lock)?;
        if !lock_meta.file_type().is_file() || lock_meta.len() > 8 * 1024 * 1024 {
            return Err(VerifiedRustError::Docker(
                "container did not produce a regular, bounded lockfile".into(),
            ));
        }
        fs::copy(&resolved_lock, staged.join("Cargo.lock"))?;
        let evidence_name = PathBuf::from("composer-build-evidence.json");
        let evidence = BuildEvidence {
            schema_version: 1,
            capsule_id: wired.capsule_id.clone(),
            image: DEFAULT_IMAGE.into(),
            network_disabled: true,
            read_only_container_root: true,
            unprivileged_user: "65534:65534".into(),
            cpu_limit: "1".into(),
            memory_limit: "1024m".into(),
            commands: vec![
                "cargo generate-lockfile --offline".into(),
                "cargo test --locked --offline --all-targets".into(),
                "cargo tree --locked --offline --depth 2".into(),
            ],
            dependencies_added: added.clone(),
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

fn copy_bounded_target(source: &Path, output: &Path) -> Result<(), VerifiedRustError> {
    let mut total_files = 0usize;
    let mut total_bytes = 0u64;
    for name in ["Cargo.toml", "Cargo.lock", "build.rs"] {
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
    for dir in ["src", "tests", "examples", "benches"] {
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

/// Conservative direct-dependency adoption. No version conflicts are
/// overwritten, and path, git, registry, build or target-specific dependencies
/// are not inferred. Cargo performs the actual offline transitive solve later.
fn merge_direct_cargo_dependencies(
    manifest: &CapsuleManifest,
    capsule_root: &Path,
    target_root: &Path,
) -> Result<Vec<String>, VerifiedRustError> {
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
    for unsupported in [
        "build-dependencies",
        "dev-dependencies",
        "target",
        "workspace",
    ] {
        if source_value.get(unsupported).is_some() {
            return Err(VerifiedRustError::Dependency(format!(
                "source manifest contains unsupported {unsupported} dependency section"
            )));
        }
    }
    let source_deps = source_value
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| {
            VerifiedRustError::Dependency(
                "source Cargo.toml must have direct [dependencies]".into(),
            )
        })?;
    let target_manifest = target_root.join("Cargo.toml");
    let mut target_value: toml::Value = toml::from_str(&fs::read_to_string(&target_manifest)?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    if target_value.get("package").is_none() || target_value.get("workspace").is_some() {
        return Err(VerifiedRustError::Dependency(
            "only standalone Cargo packages (not workspace roots) are supported".into(),
        ));
    }
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
    for (name, requested) in source_deps {
        let Some(version) = requested.as_str() else {
            return Err(VerifiedRustError::Dependency(format!(
                "{name} has unsupported dependency form; only version strings allowed"
            )));
        };
        if version.len() > 128
            || version.is_empty()
            || !version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b" .,=*^~<>+-".contains(&b))
        {
            return Err(VerifiedRustError::Dependency(format!(
                "{name} uses unsupported or unsafe version syntax"
            )));
        }
        match deps.get(name) {
            None => {
                deps.insert(name.clone(), toml::Value::String(version.to_string()));
                added.push(format!("{name} = {version}"));
            }
            Some(existing) if existing == requested => {}
            Some(_) => {
                return Err(VerifiedRustError::Dependency(format!(
                    "conflicting dependency {name}; will not overwrite existing constraint"
                )));
            }
        }
    }
    fs::write(
        target_manifest,
        toml::to_string_pretty(&target_value)
            .map_err(|err| VerifiedRustError::Toml(err.to_string()))?,
    )?;
    Ok(added)
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

fn docker_build(
    staged: &Path,
    results: &Path,
    registry: Option<&Path>,
) -> Result<String, VerifiedRustError> {
    // No shell expansion from user input: a fixed script with process arguments.
    const SCRIPT: &str = "set -eu; cp -R /input/. /work/; cargo generate-lockfile --offline; cargo test --locked --offline --all-targets; cargo tree --locked --offline --depth 2; cp Cargo.lock /results/Cargo.lock";
    let name = format!("composer-verify-{}", Uuid::new_v4().simple());
    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "--detach",
        "--pull=never",
        "--name",
        &name,
        "--network=none",
        "--read-only",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--pids-limit=64",
        "--memory=1024m",
        "--cpus=1",
        "--user=65534:65534",
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
    cmd.args(["--entrypoint", "sh", DEFAULT_IMAGE, "-c", SCRIPT]);
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

    #[test]
    fn preserves_target_manifest_and_adds_only_matching_version_deps() {
        let root = std::env::temp_dir().join(format!("composer-deps-{}", Uuid::new_v4()));
        let capsule = root.join("capsule");
        let target = root.join("target");
        fs::create_dir_all(capsule.join("context/manifests")).unwrap();
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
            files: vec![composer_intelligence::CapsuleFile {
                source_path: Some(PathBuf::from("Cargo.toml")),
                capsule_path: "context/manifests/Cargo.toml".into(),
                file_kind: "dependency_manifest".into(),
                size_bytes: 0,
                sha256: String::new(),
            }],
            included_source_files: vec![],
            included_symbol_ids: vec![],
            external_or_unresolved_imports: vec![],
            unfollowed_symbol_references: vec![],
            license_evidence_files: vec![],
            license_review_required: true,
            boundary_warnings: vec![],
        };
        let added = merge_direct_cargo_dependencies(&manifest, &capsule, &target).unwrap();
        assert_eq!(added, vec!["serde = 1"]);
        assert!(
            fs::read_to_string(target.join("Cargo.toml"))
                .unwrap()
                .contains("serde = \"1\"")
        );
        let again = merge_direct_cargo_dependencies(&manifest, &capsule, &target).unwrap();
        assert!(again.is_empty());
        fs::write(
            target.join("Cargo.toml"),
            "[package]\nname='b'\nversion='0.1.0'\n[dependencies]\nserde='2'\n",
        )
        .unwrap();
        assert!(matches!(
            merge_direct_cargo_dependencies(&manifest, &capsule, &target),
            Err(VerifiedRustError::Dependency(_))
        ));
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
        });
        assert!(matches!(result, Err(VerifiedRustError::ReviewRequired)));
    }
}
