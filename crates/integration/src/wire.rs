//! Deliberately narrow source-level integration for audited, standalone Rust modules.
//! Does not execute code, install packages, or establish license clearance.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use composer_intelligence::SourceLanguage;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    CompatibilityError, CompatibilityLevel, RequirementStatus, VerificationError, plan_integration,
    verify_capsule,
};

#[derive(Debug, Clone)]
pub struct RustWiringOptions {
    pub capsule_root: PathBuf,
    pub target_root: PathBuf,
    /// Must be explicitly supplied by the caller after independent source/license review.
    pub license_reviewed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustWiringResult {
    pub module_name: String,
    pub source_module: PathBuf,
    pub entrypoint: PathBuf,
    pub entrypoint_backup: PathBuf,
    pub capsule_id: String,
    pub build_or_test_commands_executed: bool,
    pub status: String,
}

#[derive(Debug, Error)]
pub enum RustWiringError {
    #[error("license/source review is required; explicitly pass --license-reviewed")]
    ReviewRequired,
    #[error("unsupported source-level integration: {0}")]
    Unsupported(String),
    #[error("source-level integration target already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("unsafe target component (symlink or non-regular file): {0}")]
    UnsafeTarget(PathBuf),
    #[error("compatibility planning failed: {0}")]
    Compatibility(#[from] CompatibilityError),
    #[error("capsule verification failed: {0}")]
    Verification(#[from] VerificationError),
    #[error("filesystem operation failed: {0}")]
    Filesystem(#[from] std::io::Error),
}

/// Opt-in Rust source wiring. Only a self-contained, single-file capsule with
/// already-declared identical dependencies can be attached to a Rust crate.
/// Untrusted source is never executed by Composer. Compilation and tests require
/// a separately approved, isolated execution environment.
pub fn wire_rust_module(options: &RustWiringOptions) -> Result<RustWiringResult, RustWiringError> {
    wire_rust_module_internal(options, false)
}

// This internal-only path is called on a disposable target that must pass
// offline compilation and tests before it can be published. It can defer
// heuristic unresolved call references to the Rust compiler.
pub(crate) fn wire_rust_module_for_verified_sandbox(
    options: &RustWiringOptions,
) -> Result<RustWiringResult, RustWiringError> {
    wire_rust_module_internal(options, true)
}

fn wire_rust_module_internal(
    options: &RustWiringOptions,
    defer_calls_to_compiler: bool,
) -> Result<RustWiringResult, RustWiringError> {
    if !options.license_reviewed {
        return Err(RustWiringError::ReviewRequired);
    }
    let verified = verify_capsule(&options.capsule_root)?;
    let plan = plan_integration(&options.capsule_root, &options.target_root)?;
    // Verified-only wiring can tolerate heuristic name collisions because
    // the new source resides in a distinct Rust module and must pass actual
    // compilation/tests before publishing. The unverified path remains strict.
    // Verified mode can defer dependency inventory findings to Cargo's
    // offline compiler/lockfile checks. Unverified mode retains strict gates.
    if (!defer_calls_to_compiler
        && plan
            .dependencies
            .iter()
            .any(|d| d.status != RequirementStatus::AlreadyPresent))
        || (!defer_calls_to_compiler
            && (plan.level != CompatibilityLevel::Compatible || !plan.symbol_collisions.is_empty()))
        || matches!(
            plan.level,
            CompatibilityLevel::Incompatible | CompatibilityLevel::RequiresAdapter
        )
    {
        return Err(RustWiringError::Unsupported(
            "incompatible language, unresolved dependencies, or unsafe unverified collision".into(),
        ));
    }
    if plan.source_languages != [SourceLanguage::Rust]
        || !plan.target.languages.contains(&SourceLanguage::Rust)
    {
        return Err(RustWiringError::Unsupported(
            "only Rust-to-Rust module wiring is supported".into(),
        ));
    }
    // Inline Rust test modules often use "use super::*", which the
    // heuristic import resolver cannot prove. Only the isolated
    // compiler-verified mode may defer these local imports to rustc.
    if verified
        .manifest
        .external_or_unresolved_imports
        .iter()
        .any(|import| {
            !defer_calls_to_compiler
                || !(import.contains("-> super::") || import.contains("-> self::"))
        })
        || (!defer_calls_to_compiler && !verified.manifest.unfollowed_symbol_references.is_empty())
    {
        return Err(RustWiringError::Unsupported(
            "capsule has unresolved imports or unfollowed symbol references".into(),
        ));
    }

    let sources: Vec<_> = verified
        .manifest
        .files
        .iter()
        .filter(|file| file.file_kind == "source")
        .collect();
    if sources.len() != 1 {
        return Err(RustWiringError::Unsupported(
            "exactly one source file is required".into(),
        ));
    }
    let source = sources[0];
    let source_path = source.source_path.as_ref().ok_or_else(|| {
        RustWiringError::Unsupported("source entry is missing its original path".into())
    })?;
    if source_path.extension().and_then(|s| s.to_str()) != Some("rs") {
        return Err(RustWiringError::Unsupported(
            "source must be a .rs file".into(),
        ));
    }

    let capability_id = &verified.manifest.capability_id;
    if capability_id.is_empty()
        || capability_id.len() > 64
        || !capability_id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err(RustWiringError::Unsupported(
            "capability id must be 1-64 lowercase ASCII letters, digits, hyphens or underscores"
                .into(),
        ));
    }

    let bytes = fs::read(verified.root.join(&source.capsule_path))?;
    let content = std::str::from_utf8(&bytes)
        .map_err(|_| RustWiringError::Unsupported("source is not UTF-8".into()))?;
    // Reject crate-root sensitive references and file inclusion even in the
    // sandbox. Verified mode may accept inline modules and super:: because
    // they are rechecked by the compiler and target tests before publication.
    for unsupported in [
        "crate::",
        "include!",
        "include_str!",
        "include_bytes!",
        "#[path",
        "#[macro_export",
        "#[global_allocator",
        "#[no_mangle",
        "extern crate ",
    ] {
        if content.contains(unsupported) {
            return Err(RustWiringError::Unsupported(format!(
                "source contains path-sensitive construct: {unsupported}"
            )));
        }
    }
    if !defer_calls_to_compiler {
        for unsupported in ["super::", "mod "] {
            if content.contains(unsupported) {
                return Err(RustWiringError::Unsupported(format!(
                    "unverified wiring rejects module-sensitive construct: {unsupported}"
                )));
            }
        }
    }

    let module_name = format!("composer_{}", capability_id.replace('-', "_"));
    let target = options.target_root.canonicalize()?;
    ensure_regular_directory(&target.join("src"))?;
    ensure_regular_file(&target.join("Cargo.toml"))?;
    let lib = target.join("src/lib.rs");
    let main = target.join("src/main.rs");
    let (entrypoint, declaration) = if lib.is_file() {
        (lib, format!("pub mod {module_name};"))
    } else if main.is_file() {
        (main, format!("mod {module_name};"))
    } else {
        return Err(RustWiringError::Unsupported(
            "target must contain src/lib.rs or src/main.rs".into(),
        ));
    };
    ensure_regular_file(&entrypoint)?;
    let original = fs::read(&entrypoint)?;
    let original_text = std::str::from_utf8(&original)
        .map_err(|_| RustWiringError::Unsupported("entrypoint is not UTF-8".into()))?;
    if original_text.contains(&declaration)
        || original_text.contains(&format!("mod {module_name};"))
    {
        return Err(RustWiringError::Unsupported(
            "target already declares this module".into(),
        ));
    }

    let module_path = target.join("src").join(format!("{module_name}.rs"));
    let backup = target
        .join("src")
        .join(format!(".{module_name}.root-backup"));
    for file in [&module_path, &backup] {
        if fs::symlink_metadata(file).is_ok() {
            return Err(RustWiringError::AlreadyExists(file.to_path_buf()));
        }
    }

    let mut updated = original.clone();
    if !updated.is_empty() && !updated.ends_with(b"\n") {
        updated.push(b'\n');
    }
    updated.extend_from_slice(
        format!(
            "\n// Software Composer: reviewed capsule {}\n{declaration}\n",
            verified.manifest.capsule_id
        )
        .as_bytes(),
    );

    // Create-new semantics prevent accidental clobbering. The persistent
    // entrypoint backup is a recovery artifact if a machine loses power.
    let mut backup_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)?;
    backup_file.write_all(&original)?;
    backup_file.sync_all()?;

    let create_module = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&module_path)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        });
    if let Err(err) = create_module {
        let _ = fs::remove_file(&module_path);
        let _ = fs::remove_file(&backup);
        return Err(err.into());
    }

    if let Err(err) = fs::write(&entrypoint, &updated) {
        let _ = fs::write(&entrypoint, &original);
        let _ = fs::remove_file(&module_path);
        return Err(err.into());
    }
    let observed = fs::read(&entrypoint);
    if !matches!(observed, Ok(ref actual) if actual == &updated) {
        let _ = fs::write(&entrypoint, &original);
        let _ = fs::remove_file(&module_path);
        return Err(RustWiringError::Unsupported(
            "entrypoint post-write verification failed; attempted rollback".into(),
        ));
    }

    Ok(RustWiringResult {
        module_name,
        source_module: module_path,
        entrypoint,
        entrypoint_backup: backup,
        capsule_id: verified.manifest.capsule_id,
        build_or_test_commands_executed: false,
        status: "source_wired_unverified_build".into(),
    })
}

fn ensure_regular_file(path: &Path) -> Result<(), RustWiringError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(RustWiringError::UnsafeTarget(path.to_path_buf()));
    }
    Ok(())
}

fn ensure_regular_directory(path: &Path) -> Result<(), RustWiringError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(RustWiringError::UnsafeTarget(path.to_path_buf()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use composer_intelligence::{CapsuleOptions, analyze_repository, create_feature_capsule};
    use uuid::Uuid;

    fn fixture(target_dependencies: &str) -> (PathBuf, PathBuf, PathBuf) {
        let workspace = std::env::temp_dir().join(format!("composer-wiring-{}", Uuid::new_v4()));
        let source = workspace.join("source");
        let target = workspace.join("target");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(target.join("src")).unwrap();
        fs::write(source.join("Cargo.toml"),
            "[package]\nname = \"source\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n[dependencies]\nserde = \"1\"\n").unwrap();
        fs::write(source.join("LICENSE"), "MIT License").unwrap();
        fs::write(
            source.join("src/feature.rs"),
            "pub fn feature_entry() -> i32 { 42 }\n",
        )
        .unwrap();
        fs::write(target.join("Cargo.toml"), format!(
            "[package]\nname = \"target\"\nversion = \"0.1.0\"\n[dependencies]\n{target_dependencies}"
        )).unwrap();
        fs::write(target.join("src/lib.rs"), "pub fn existing() {}\n").unwrap();
        let graph = analyze_repository(&source).unwrap();
        let seed = graph
            .symbols
            .iter()
            .find(|s| s.name == "feature_entry")
            .unwrap();
        let capsule = workspace.join("capsule");
        create_feature_capsule(
            &graph,
            &CapsuleOptions {
                capability_id: "feature-entry".into(),
                description: "Feature entry".into(),
                seed_symbol_ids: vec![seed.id.clone()],
                output_dir: capsule.clone(),
            },
        )
        .unwrap();
        (workspace, capsule, target)
    }

    #[test]
    fn wires_a_verified_single_file_and_preserves_backup() {
        let (workspace, capsule, target) = fixture("serde = \"1\"\n");
        let outcome = wire_rust_module(&RustWiringOptions {
            capsule_root: capsule,
            target_root: target.clone(),
            license_reviewed: true,
        })
        .unwrap();
        assert_eq!(outcome.module_name, "composer_feature_entry");
        assert!(outcome.source_module.is_file());
        assert!(outcome.entrypoint_backup.is_file());
        assert!(
            fs::read_to_string(&outcome.entrypoint)
                .unwrap()
                .contains("pub mod composer_feature_entry;")
        );
        assert!(!outcome.build_or_test_commands_executed);
        // Compile only this trusted, synthetic fixture. Untrusted source is never
        // compiled automatically by the production wiring command.
        let compiled = std::process::Command::new("rustc")
            .arg("--edition=2024")
            .arg("--crate-type=lib")
            .arg(&outcome.entrypoint)
            .arg("-o")
            .arg(workspace.join("wired.rlib"))
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn rejects_without_explicit_review_and_never_changes_target() {
        let (workspace, capsule, target) = fixture("serde = \"1\"\n");
        let result = wire_rust_module(&RustWiringOptions {
            capsule_root: capsule,
            target_root: target.clone(),
            license_reviewed: false,
        });
        assert!(matches!(result, Err(RustWiringError::ReviewRequired)));
        assert!(!target.join("src/composer_feature_entry.rs").exists());
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn refuses_missing_dependency_and_preserves_entrypoint() {
        let (workspace, capsule, target) = fixture("");
        let entry = fs::read(target.join("src/lib.rs")).unwrap();
        let result = wire_rust_module(&RustWiringOptions {
            capsule_root: capsule,
            target_root: target.clone(),
            license_reviewed: true,
        });
        assert!(matches!(result, Err(RustWiringError::Unsupported(_))));
        assert_eq!(fs::read(target.join("src/lib.rs")).unwrap(), entry);
        assert!(!target.join("src/composer_feature_entry.rs").exists());
        let _ = fs::remove_dir_all(workspace);
    }
}
