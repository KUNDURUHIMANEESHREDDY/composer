use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

use composer_intelligence::CapsuleManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const CAPSULE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleVerification {
    pub schema_version: u32,
    pub capsule_id: String,
    pub verified_files: usize,
    pub source_files: usize,
    pub dependency_manifests: usize,
    pub license_evidence_files: usize,
    pub total_bytes: u64,
    pub manifest_sha256: String,
    /// Hashes prove consistency with the manifest; they are not a signature
    /// and do not establish who produced the capsule.
    pub authenticity_attested: bool,
}

#[derive(Debug, Clone)]
pub struct VerifiedCapsule {
    pub root: PathBuf,
    pub manifest: CapsuleManifest,
    pub verification: CapsuleVerification,
}

#[derive(Debug, Error)]
pub enum VerificationError {
    #[error("capsule directory does not exist: {0}")]
    MissingCapsule(PathBuf),
    #[error("capsule path is not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("capsule manifest is missing: {0}")]
    MissingManifest(PathBuf),
    #[error("unsupported Feature Capsule schema version: {0}")]
    UnsupportedSchema(u32),
    #[error("capsule manifest is invalid: {0}")]
    InvalidManifest(#[from] serde_json::Error),
    #[error("capsule file path is unsafe: {0}")]
    UnsafePath(PathBuf),
    #[error("capsule contains a duplicate path: {0}")]
    DuplicatePath(PathBuf),
    #[error("capsule file is missing or is not a regular file: {0}")]
    MissingFile(PathBuf),
    #[error("capsule file size mismatch for {path}: expected {expected}, found {actual}")]
    SizeMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    #[error("capsule file hash mismatch for {0}")]
    HashMismatch(PathBuf),
    #[error("included source path has no matching source artifact: {0}")]
    MissingSourceEntry(PathBuf),
    #[error("license evidence path has no matching artifact: {0}")]
    MissingLicenseEntry(PathBuf),
    #[error("capsule contains no source files")]
    EmptyCapsule,
    #[error("filesystem operation failed: {0}")]
    Filesystem(#[from] std::io::Error),
}

pub fn verify_capsule(capsule_root: &Path) -> Result<VerifiedCapsule, VerificationError> {
    if !capsule_root.exists() {
        return Err(VerificationError::MissingCapsule(
            capsule_root.to_path_buf(),
        ));
    }
    if !capsule_root.is_dir() {
        return Err(VerificationError::NotADirectory(capsule_root.to_path_buf()));
    }

    let root = capsule_root.canonicalize()?;
    let manifest_path = root.join("manifest.json");
    let manifest_metadata = fs::symlink_metadata(&manifest_path)
        .map_err(|_| VerificationError::MissingManifest(manifest_path.clone()))?;
    if !manifest_metadata.file_type().is_file() {
        return Err(VerificationError::MissingManifest(manifest_path));
    }

    let manifest_bytes = fs::read(&manifest_path)?;
    let manifest: CapsuleManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.schema_version != CAPSULE_SCHEMA_VERSION {
        return Err(VerificationError::UnsupportedSchema(
            manifest.schema_version,
        ));
    }

    let mut seen_paths = HashSet::new();
    let mut verified_files = 0usize;
    let mut source_files = 0usize;
    let mut dependency_manifests = 0usize;
    let mut license_evidence_files = 0usize;
    let mut total_bytes = 0u64;

    for entry in &manifest.files {
        let relative = safe_relative_path(&entry.capsule_path)?;
        let key = entry.capsule_path.replace('\\', "/").to_ascii_lowercase();
        if !seen_paths.insert(key) {
            return Err(VerificationError::DuplicatePath(relative));
        }

        reject_symlink_components(&root, &relative)?;
        let source = root.join(&relative);
        let metadata = fs::symlink_metadata(&source)
            .map_err(|_| VerificationError::MissingFile(relative.clone()))?;
        if !metadata.file_type().is_file() {
            return Err(VerificationError::MissingFile(relative));
        }

        let bytes = fs::read(&source)?;
        if bytes.len() as u64 != entry.size_bytes {
            return Err(VerificationError::SizeMismatch {
                path: relative,
                expected: entry.size_bytes,
                actual: bytes.len() as u64,
            });
        }

        let actual_hash = sha256_hex(&bytes);
        if !actual_hash.eq_ignore_ascii_case(&entry.sha256) {
            return Err(VerificationError::HashMismatch(relative));
        }

        match entry.file_kind.as_str() {
            "source" => source_files += 1,
            "dependency_manifest" => dependency_manifests += 1,
            "license_evidence" => license_evidence_files += 1,
            _ => {}
        }
        verified_files += 1;
        total_bytes += bytes.len() as u64;
    }

    if source_files == 0 || manifest.included_source_files.is_empty() {
        return Err(VerificationError::EmptyCapsule);
    }

    for source_path in &manifest.included_source_files {
        validate_relative_path(source_path)?;
        let found = manifest.files.iter().any(|entry| {
            entry.file_kind == "source" && entry.source_path.as_ref() == Some(source_path)
        });
        if !found {
            return Err(VerificationError::MissingSourceEntry(source_path.clone()));
        }
    }

    for license_path in &manifest.license_evidence_files {
        validate_relative_path(license_path)?;
        let found = manifest.files.iter().any(|entry| {
            entry.file_kind == "license_evidence"
                && entry.source_path.as_ref() == Some(license_path)
        });
        if !found {
            return Err(VerificationError::MissingLicenseEntry(license_path.clone()));
        }
    }

    let verification = CapsuleVerification {
        schema_version: CAPSULE_SCHEMA_VERSION,
        capsule_id: manifest.capsule_id.clone(),
        verified_files,
        source_files,
        dependency_manifests,
        license_evidence_files,
        total_bytes,
        manifest_sha256: sha256_hex(&manifest_bytes),
        authenticity_attested: false,
    };

    Ok(VerifiedCapsule {
        root,
        manifest,
        verification,
    })
}

fn safe_relative_path(value: &str) -> Result<PathBuf, VerificationError> {
    let path = PathBuf::from(value);
    validate_relative_path(&path)?;
    Ok(path)
}

fn validate_relative_path(path: &Path) -> Result<(), VerificationError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(VerificationError::UnsafePath(path.to_path_buf()));
    }
    Ok(())
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<(), VerificationError> {
    let mut current = root.to_path_buf();
    for (index, component) in relative.components().enumerate() {
        let Component::Normal(name) = component else {
            return Err(VerificationError::UnsafePath(relative.to_path_buf()));
        };
        current.push(name);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| VerificationError::MissingFile(relative.to_path_buf()))?;
        if metadata.file_type().is_symlink() {
            return Err(VerificationError::UnsafePath(current));
        }
        if index + 1 < relative.components().count() && !metadata.is_dir() {
            return Err(VerificationError::MissingFile(relative.to_path_buf()));
        }
    }
    let canonical = current.canonicalize()?;
    if !canonical.starts_with(root) {
        return Err(VerificationError::UnsafePath(relative.to_path_buf()));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use composer_intelligence::{CapsuleOptions, analyze_repository, create_feature_capsule};
    use uuid::Uuid;

    use super::*;

    fn fixture() -> (PathBuf, PathBuf) {
        let workspace = std::env::temp_dir().join(format!("composer-verify-{}", Uuid::new_v4()));
        let source = workspace.join("source");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(
            source.join("Cargo.toml"),
            "[package]\nname = \"verification_fixture\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n",
        )
        .unwrap();
        fs::write(source.join("LICENSE-MIT"), "MIT License\n").unwrap();
        fs::write(source.join("src/lib.rs"), "pub fn verified_feature() {}\n").unwrap();

        let graph = analyze_repository(&source).unwrap();
        let seed = graph
            .symbols
            .iter()
            .find(|symbol| symbol.name == "verified_feature")
            .unwrap();
        let capsule = workspace.join("capsule");
        create_feature_capsule(
            &graph,
            &CapsuleOptions {
                capability_id: "verified-feature".to_owned(),
                description: "Verified feature".to_owned(),
                seed_symbol_ids: vec![seed.id.clone()],
                output_dir: capsule.clone(),
            },
        )
        .unwrap();

        (workspace, capsule)
    }

    #[test]
    fn verifies_manifest_file_hashes_and_source_entries() {
        let (workspace, capsule) = fixture();
        let result = verify_capsule(&capsule).unwrap();

        assert_eq!(result.verification.source_files, 1);
        assert_eq!(result.verification.dependency_manifests, 1);
        assert_eq!(result.verification.license_evidence_files, 1);
        assert_eq!(result.verification.verified_files, 3);
        assert!(!result.verification.authenticity_attested);
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn detects_capsule_file_tampering() {
        let (workspace, capsule) = fixture();
        let source_file = capsule.join("source/src/lib.rs");
        fs::write(
            &source_file,
            "pub fn modified_after_capsule_creation() {}\n",
        )
        .unwrap();

        let result = verify_capsule(&capsule);
        assert!(matches!(
            result,
            Err(VerificationError::HashMismatch(_) | VerificationError::SizeMismatch { .. })
        ));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn rejects_manifest_path_traversal() {
        let (workspace, capsule) = fixture();
        let manifest_path = capsule.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["files"][0]["capsule_path"] =
            serde_json::Value::String("../escape.txt".to_owned());
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let result = verify_capsule(&capsule);
        assert!(
            matches!(result, Err(VerificationError::UnsafePath(path)) if path == Path::new("../escape.txt"))
        );
        let _ = fs::remove_dir_all(workspace);
    }
}
