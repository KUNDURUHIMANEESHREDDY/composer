use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::{BoundaryError, FeatureBoundary, RepositoryGraph, analyze_feature_boundary};

#[derive(Debug, Clone)]
pub struct CapsuleOptions {
    pub capability_id: String,
    pub description: String,
    pub seed_symbol_ids: Vec<String>,
    pub output_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleFile {
    pub source_path: Option<PathBuf>,
    pub capsule_path: String,
    pub file_kind: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleManifest {
    pub schema_version: u32,
    pub capsule_id: String,
    pub capability_id: String,
    pub description: String,
    pub created_at_unix: u64,
    pub source_repository_url: Option<String>,
    pub source_commit_sha: Option<String>,
    pub source_tree_sha: Option<String>,
    pub dependency_closure_strategy: String,
    pub files: Vec<CapsuleFile>,
    pub included_source_files: Vec<PathBuf>,
    pub included_symbol_ids: Vec<String>,
    pub external_or_unresolved_imports: Vec<String>,
    pub unfollowed_symbol_references: Vec<String>,
    pub license_evidence_files: Vec<PathBuf>,
    pub license_review_required: bool,
    pub boundary_warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleResult {
    pub output_dir: PathBuf,
    pub manifest: CapsuleManifest,
}

#[derive(Debug, Error)]
pub enum CapsuleError {
    #[error("feature boundary could not be built: {0}")]
    Boundary(#[from] BoundaryError),
    #[error("output directory already exists: {0}")]
    OutputAlreadyExists(PathBuf),
    #[error("unsafe or invalid relative source path: {0}")]
    UnsafePath(PathBuf),
    #[error("source file is no longer a regular file: {0}")]
    SourceNotRegularFile(PathBuf),
    #[error("source and output paths must not overlap")]
    PathsOverlap,
    #[error("filesystem operation failed: {0}")]
    Filesystem(#[from] std::io::Error),
    #[error("could not serialize capsule manifest: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Create a versioned source capsule from explicitly selected seed symbols.
///
/// Whole source files are copied for a conservative dependency closure. Related
/// manifests and root-level license/notice evidence are included as context.
/// This does not claim independent buildability or legal clearance.
pub fn create_feature_capsule(
    graph: &RepositoryGraph,
    options: &CapsuleOptions,
) -> Result<CapsuleResult, CapsuleError> {
    let boundary = analyze_feature_boundary(
        graph,
        &options.capability_id,
        &options.description,
        &options.seed_symbol_ids,
    )?;

    let repository_root = graph.repository_root.canonicalize()?;
    let output_dir = absolute_path(&options.output_dir)?;
    if output_dir.exists() {
        return Err(CapsuleError::OutputAlreadyExists(output_dir));
    }
    let canonical_output_dir = resolve_output_path(&output_dir)?;
    if canonical_output_dir.starts_with(&repository_root)
        || repository_root.starts_with(&canonical_output_dir)
    {
        return Err(CapsuleError::PathsOverlap);
    }

    let parent = output_dir
        .parent()
        .ok_or_else(|| CapsuleError::UnsafePath(output_dir.clone()))?;
    fs::create_dir_all(parent)?;

    let temporary = parent.join(format!(".composer-capsule-{}.tmp", Uuid::new_v4()));
    fs::create_dir(&temporary)?;

    let result = build_capsule_in_directory(&repository_root, &boundary, &temporary);
    match result {
        Ok(manifest) => {
            if let Err(error) = fs::rename(&temporary, &output_dir) {
                let _ = fs::remove_dir_all(&temporary);
                return Err(error.into());
            }
            Ok(CapsuleResult {
                output_dir,
                manifest,
            })
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&temporary);
            Err(error)
        }
    }
}

fn build_capsule_in_directory(
    repository_root: &Path,
    boundary: &FeatureBoundary,
    output_dir: &Path,
) -> Result<CapsuleManifest, CapsuleError> {
    let mut files = Vec::new();
    let source_paths: BTreeSet<PathBuf> = boundary.included_files.iter().cloned().collect();

    for relative in &source_paths {
        ensure_safe_relative_path(relative)?;
        files.push(copy_file(
            repository_root,
            relative,
            output_dir,
            &PathBuf::from("source").join(relative),
            "source",
        )?);
    }

    let manifest_paths = relevant_manifest_paths(repository_root, &source_paths)?;
    for relative in &manifest_paths {
        ensure_safe_relative_path(relative)?;
        files.push(copy_file(
            repository_root,
            relative,
            output_dir,
            &PathBuf::from("context/manifests").join(relative),
            "dependency_manifest",
        )?);
    }

    let license_paths = root_license_evidence(repository_root)?;
    for relative in &license_paths {
        ensure_safe_relative_path(relative)?;
        files.push(copy_file(
            repository_root,
            relative,
            output_dir,
            &PathBuf::from("licenses").join(relative),
            "license_evidence",
        )?);
    }

    files.sort_by(|left, right| left.capsule_path.cmp(&right.capsule_path));
    let (source_repository_url, source_commit_sha, source_tree_sha) =
        source_git_metadata(repository_root);

    let external_or_unresolved_imports = boundary
        .unresolved_imports
        .iter()
        .map(|item| {
            format!(
                "{}:{} -> {} ({:?})",
                item.source_file.display(),
                item.line,
                item.target,
                item.resolution
            )
        })
        .collect();

    let unfollowed_symbol_references = boundary
        .unfollowed_symbol_references
        .iter()
        .map(|item| {
            format!(
                "{}:{} -> {} ({:?})",
                item.source_file.display(),
                item.line,
                item.target_name,
                item.resolution
            )
        })
        .collect();

    let manifest = CapsuleManifest {
        schema_version: 1,
        capsule_id: Uuid::new_v4().to_string(),
        capability_id: boundary.capability_id.clone(),
        description: boundary.description.clone(),
        created_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        source_repository_url,
        source_commit_sha,
        source_tree_sha,
        dependency_closure_strategy: boundary.boundary_strategy.clone(),
        files,
        included_source_files: boundary.included_files.clone(),
        included_symbol_ids: boundary.included_symbol_ids.clone(),
        external_or_unresolved_imports,
        unfollowed_symbol_references,
        license_evidence_files: license_paths,
        license_review_required: true,
        boundary_warnings: boundary.warnings.clone(),
    };

    fs::write(
        output_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

fn copy_file(
    repository_root: &Path,
    relative: &Path,
    output_root: &Path,
    capsule_relative: &Path,
    kind: &str,
) -> Result<CapsuleFile, CapsuleError> {
    ensure_safe_relative_path(relative)?;
    ensure_safe_relative_path(capsule_relative)?;

    let source = repository_root.join(relative);
    let metadata = fs::symlink_metadata(&source)?;
    if !metadata.file_type().is_file() {
        return Err(CapsuleError::SourceNotRegularFile(source));
    }
    let canonical_source = source.canonicalize()?;
    if !canonical_source.starts_with(repository_root) {
        return Err(CapsuleError::UnsafePath(relative.to_path_buf()));
    }
    reject_symlink_components(repository_root, relative)?;

    let bytes = fs::read(&source)?;
    let destination = output_root.join(capsule_relative);
    fs::create_dir_all(
        destination
            .parent()
            .ok_or_else(|| CapsuleError::UnsafePath(capsule_relative.to_path_buf()))?,
    )?;
    fs::write(&destination, &bytes)?;

    let digest = Sha256::digest(&bytes);
    Ok(CapsuleFile {
        source_path: Some(relative.to_path_buf()),
        capsule_path: capsule_relative.to_string_lossy().replace('\\', "/"),
        file_kind: kind.to_owned(),
        size_bytes: bytes.len() as u64,
        sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
    })
}

fn relevant_manifest_paths(
    repository_root: &Path,
    source_paths: &BTreeSet<PathBuf>,
) -> Result<Vec<PathBuf>, CapsuleError> {
    const NAMES: &[&str] = &[
        "Cargo.toml",
        "package.json",
        "tsconfig.json",
        "pyproject.toml",
        "requirements.txt",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "composer.json",
        "Gemfile",
        "mix.exs",
    ];

    let mut result = BTreeSet::new();
    for source in source_paths {
        let mut current = source.parent();
        while let Some(relative_dir) = current {
            let absolute_dir = repository_root.join(relative_dir);
            for name in NAMES {
                let candidate = absolute_dir.join(name);
                if candidate.is_file() {
                    result.insert(
                        candidate
                            .strip_prefix(repository_root)
                            .expect("manifest is under root")
                            .to_path_buf(),
                    );
                }
            }
            current = relative_dir.parent();
        }
        for name in NAMES {
            if repository_root.join(name).is_file() {
                result.insert(PathBuf::from(name));
            }
        }
    }
    Ok(result.into_iter().collect())
}

fn root_license_evidence(repository_root: &Path) -> Result<Vec<PathBuf>, CapsuleError> {
    let mut result = Vec::new();
    for entry in fs::read_dir(repository_root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() || file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_uppercase();
        if name == "LICENSE"
            || name.starts_with("LICENSE.")
            || name.starts_with("LICENSE-")
            || name == "COPYING"
            || name.starts_with("COPYING.")
            || name.starts_with("COPYING-")
            || name == "NOTICE"
            || name.starts_with("NOTICE.")
            || name.starts_with("NOTICE-")
            || name == "AUTHORS"
            || name == "COPYRIGHT"
        {
            result.push(PathBuf::from(entry.file_name()));
        }
    }
    result.sort();
    Ok(result)
}

fn source_git_metadata(root: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let url = git_output(root, &["remote", "get-url", "origin"])
        .and_then(|value| sanitize_remote_url(&value));
    let commit = git_output(root, &["rev-parse", "--verify", "HEAD^{commit}"]);
    let tree = git_output(root, &["rev-parse", "--verify", "HEAD^{tree}"]);
    (url, commit, tree)
}

fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn sanitize_remote_url(value: &str) -> Option<String> {
    let mut url = Url::parse(value).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    url.set_username("").ok()?;
    url.set_password(None).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn ensure_safe_relative_path(path: &Path) -> Result<(), CapsuleError> {
    if path.as_os_str().is_empty()
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(CapsuleError::UnsafePath(path.to_path_buf()));
    }
    Ok(())
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<(), CapsuleError> {
    let mut current = root.to_path_buf();
    let mut components = relative.components().peekable();

    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(CapsuleError::UnsafePath(relative.to_path_buf()));
        };
        current.push(name);
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() {
            return Err(CapsuleError::SourceNotRegularFile(current));
        }
        if components.peek().is_some() && !metadata.is_dir() {
            return Err(CapsuleError::SourceNotRegularFile(current));
        }
    }

    Ok(())
}

fn absolute_path(path: &Path) -> Result<PathBuf, CapsuleError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(normalize_absolute_path(&absolute))
}

fn resolve_output_path(path: &Path) -> Result<PathBuf, CapsuleError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let absolute = normalize_absolute_path(&absolute);

    let mut probe = absolute.as_path();
    let mut missing_components = Vec::new();
    while !probe.exists() {
        let name = probe
            .file_name()
            .ok_or_else(|| CapsuleError::UnsafePath(absolute.clone()))?;
        missing_components.push(name.to_os_string());
        probe = probe
            .parent()
            .ok_or_else(|| CapsuleError::UnsafePath(absolute.clone()))?;
    }

    let mut resolved = probe.canonicalize()?;
    for component in missing_components.iter().rev() {
        resolved.push(component);
    }

    Ok(normalize_absolute_path(&resolved))
}

fn normalize_absolute_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(name) => normalized.push(name),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use crate::{
        SourceLanguage,
        model::{
            AnalysisSummary, AnalyzedFile, ImportDependency, ImportResolution, RepositoryGraph,
            Symbol, SymbolKind, SymbolReference, SymbolResolution,
        },
    };

    use super::*;

    fn sample_graph(root: PathBuf) -> RepositoryGraph {
        let symbols = vec![
            Symbol {
                id: "sym-seed".to_owned(),
                name: "authenticate_user".to_owned(),
                kind: SymbolKind::Function,
                file_path: PathBuf::from("src/auth.rs"),
                start_byte: 0,
                end_byte: 25,
                start_line: 1,
                end_line: 2,
                parent_symbol_id: None,
            },
            Symbol {
                id: "sym-dependency".to_owned(),
                name: "verify_token".to_owned(),
                kind: SymbolKind::Function,
                file_path: PathBuf::from("src/token.rs"),
                start_byte: 0,
                end_byte: 20,
                start_line: 1,
                end_line: 2,
                parent_symbol_id: None,
            },
        ];
        RepositoryGraph {
            schema_version: 1,
            repository_root: root,
            files: vec![
                AnalyzedFile {
                    path: PathBuf::from("src/auth.rs"),
                    language: SourceLanguage::Rust,
                    size_bytes: 50,
                    line_count: 2,
                    has_syntax_errors: false,
                    syntax_error_count: 0,
                },
                AnalyzedFile {
                    path: PathBuf::from("src/token.rs"),
                    language: SourceLanguage::Rust,
                    size_bytes: 30,
                    line_count: 2,
                    has_syntax_errors: false,
                    syntax_error_count: 0,
                },
            ],
            symbols,
            imports: vec![ImportDependency {
                source_file: PathBuf::from("src/auth.rs"),
                target: "crate::token".to_owned(),
                kind: "use_declaration".to_owned(),
                line: 1,
                resolution: ImportResolution::Local,
                resolved_file: Some(PathBuf::from("src/token.rs")),
            }],
            references: vec![SymbolReference {
                source_file: PathBuf::from("src/auth.rs"),
                from_symbol_id: Some("sym-seed".to_owned()),
                target_name: "verify_token".to_owned(),
                line: 2,
                resolved_symbol_id: Some("sym-dependency".to_owned()),
                resolution: SymbolResolution::UniqueNameMatch,
            }],
            summary: AnalysisSummary::default(),
        }
    }

    #[test]
    fn creates_capsule_with_hashes_and_license_evidence() {
        let temporary = std::env::temp_dir().join(format!("composer-capsule-{}", Uuid::new_v4()));
        let source = temporary.join("source-repo");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(source.join("src/auth.rs"), "pub fn authenticate_user() {}").unwrap();
        fs::write(source.join("src/token.rs"), "pub fn verify_token() {}").unwrap();
        fs::write(source.join("Cargo.toml"), "[package]\nname = \"sample\"\n").unwrap();
        fs::write(source.join("LICENSE-MIT"), "MIT License\n").unwrap();
        fs::write(source.join("LICENSE-APACHE"), "Apache License\n").unwrap();

        let options = CapsuleOptions {
            capability_id: "authentication".to_owned(),
            description: "Authenticate users".to_owned(),
            seed_symbol_ids: vec!["sym-seed".to_owned()],
            output_dir: temporary.join("capsule"),
        };
        let result = create_feature_capsule(&sample_graph(source), &options).unwrap();

        assert_eq!(result.manifest.included_source_files.len(), 2);
        assert_eq!(
            result.manifest.license_evidence_files,
            vec![
                PathBuf::from("LICENSE-APACHE"),
                PathBuf::from("LICENSE-MIT")
            ]
        );
        assert!(result.manifest.license_review_required);
        assert!(
            result
                .manifest
                .files
                .iter()
                .all(|file| file.sha256.len() == 64)
        );
        assert!(result.output_dir.join("manifest.json").is_file());
        assert!(result.output_dir.join("licenses/LICENSE-APACHE").is_file());
        assert!(result.output_dir.join("licenses/LICENSE-MIT").is_file());
        assert!(result.output_dir.join("source/src/auth.rs").is_file());
        let _ = fs::remove_dir_all(temporary);
    }

    #[test]
    fn refuses_to_overwrite_existing_output() {
        let temporary = std::env::temp_dir().join(format!("composer-capsule-{}", Uuid::new_v4()));
        let source = temporary.join("source-repo");
        let output = temporary.join("capsule");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(&output).unwrap();
        fs::write(source.join("src/auth.rs"), "pub fn authenticate_user() {}").unwrap();

        let options = CapsuleOptions {
            capability_id: "authentication".to_owned(),
            description: "Authenticate users".to_owned(),
            seed_symbol_ids: vec!["sym-seed".to_owned()],
            output_dir: output.clone(),
        };
        let result = create_feature_capsule(&sample_graph(source), &options);
        assert!(matches!(result, Err(CapsuleError::OutputAlreadyExists(path)) if path == output));
        let _ = fs::remove_dir_all(temporary);
    }

    #[test]
    fn refuses_output_inside_source_tree_even_with_parent_components() {
        let temporary = std::env::temp_dir().join(format!("composer-capsule-{}", Uuid::new_v4()));
        let source = temporary.join("source-repo");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(source.join("src/auth.rs"), "pub fn authenticate_user() {}").unwrap();

        let output = source.join("nested").join("..").join("capsule");
        let options = CapsuleOptions {
            capability_id: "authentication".to_owned(),
            description: "Authenticate users".to_owned(),
            seed_symbol_ids: vec!["sym-seed".to_owned()],
            output_dir: output,
        };
        let result = create_feature_capsule(&sample_graph(source.clone()), &options);

        assert!(matches!(result, Err(CapsuleError::PathsOverlap)));
        assert!(!source.join("capsule").exists());
        let _ = fs::remove_dir_all(temporary);
    }
}
