use std::{
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    CompatibilityError, CompatibilityPlan, VerificationError, plan_integration, verify_capsule,
};

#[derive(Debug, Clone)]
pub struct IntegrationOptions {
    pub capsule_root: PathBuf,
    pub target_root: PathBuf,
    /// A new directory relative to the target root. Defaults to
    /// .composer/integrations/<capability-id>.
    pub destination: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationResult {
    pub destination: PathBuf,
    pub copied_capsule_files: usize,
    pub staged_verification: crate::CapsuleVerification,
    pub plan: CompatibilityPlan,
    pub build_or_test_commands_executed: bool,
}

#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("compatibility planning failed: {0}")]
    Compatibility(#[from] CompatibilityError),
    #[error("capsule verification failed: {0}")]
    Verification(#[from] VerificationError),
    #[error("target directory does not exist: {0}")]
    MissingTarget(PathBuf),
    #[error("target destination must be a relative path inside the target repository: {0}")]
    UnsafeDestination(PathBuf),
    #[error("integration destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("integration destination overlaps the source capsule")]
    DestinationOverlapsCapsule,
    #[error("target path contains a symlink or non-directory component: {0}")]
    UnsafeTargetComponent(PathBuf),
    #[error("filesystem operation failed: {0}")]
    Filesystem(#[from] std::io::Error),
    #[error("could not serialize integration artifacts: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub fn apply_integration(
    options: &IntegrationOptions,
) -> Result<IntegrationResult, IntegrationError> {
    if !options.target_root.exists() {
        return Err(IntegrationError::MissingTarget(options.target_root.clone()));
    }

    let plan = plan_integration(&options.capsule_root, &options.target_root)?;
    let target_root = options.target_root.canonicalize()?;
    let verified_capsule = verify_capsule(&options.capsule_root)?;
    let relative_destination = options
        .destination
        .clone()
        .unwrap_or_else(|| plan.recommended_destination.clone());
    validate_destination(&relative_destination)?;

    let destination = target_root.join(&relative_destination);
    if destination.starts_with(&verified_capsule.root)
        || verified_capsule.root.starts_with(&destination)
    {
        return Err(IntegrationError::DestinationOverlapsCapsule);
    }

    ensure_target_parent(&target_root, relative_destination.parent())?;
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err(IntegrationError::DestinationExists(destination)),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let parent = destination
        .parent()
        .ok_or_else(|| IntegrationError::UnsafeDestination(relative_destination.clone()))?;
    let temporary = parent.join(format!(".composer-integration-{}.tmp", Uuid::new_v4()));
    fs::create_dir(&temporary)?;

    let staged_result = build_staged_integration(&verified_capsule, &plan, &temporary);

    match staged_result {
        Ok((copied_capsule_files, staged_verification)) => {
            if let Err(error) = fs::rename(&temporary, &destination) {
                let _ = fs::remove_dir_all(&temporary);
                return Err(error.into());
            }
            Ok(IntegrationResult {
                destination,
                copied_capsule_files,
                staged_verification,
                plan,
                build_or_test_commands_executed: false,
            })
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&temporary);
            Err(error)
        }
    }
}

fn build_staged_integration(
    verified_capsule: &crate::VerifiedCapsule,
    plan: &CompatibilityPlan,
    staging_root: &Path,
) -> Result<(usize, crate::CapsuleVerification), IntegrationError> {
    let capsule_destination = staging_root.join("capsule");
    fs::create_dir(&capsule_destination)?;

    for file in &verified_capsule.manifest.files {
        let relative = safe_capsule_relative_path(&file.capsule_path)?;
        let source = verified_capsule.root.join(&relative);
        let destination = capsule_destination.join(&relative);

        let metadata = fs::symlink_metadata(&source)?;
        if !metadata.file_type().is_file() {
            return Err(VerificationError::MissingFile(relative).into());
        }

        let bytes = fs::read(&source)?;
        let parent = destination
            .parent()
            .ok_or_else(|| IntegrationError::UnsafeDestination(relative.clone()))?;
        fs::create_dir_all(parent)?;
        fs::write(&destination, bytes)?;
    }

    fs::copy(
        verified_capsule.root.join("manifest.json"),
        capsule_destination.join("manifest.json"),
    )?;

    fs::write(
        staging_root.join("integration-plan.json"),
        serde_json::to_vec_pretty(plan)?,
    )?;
    fs::write(
        staging_root.join("adapter-contract.json"),
        serde_json::to_vec_pretty(&plan.adapter_contract)?,
    )?;
    fs::write(
        staging_root.join("capsule-verification.json"),
        serde_json::to_vec_pretty(&verified_capsule.verification)?,
    )?;
    fs::write(
        staging_root.join("INTEGRATION.md"),
        integration_readme(plan),
    )?;

    let staged = verify_capsule(&capsule_destination)?;
    if staged.verification.manifest_sha256 != verified_capsule.verification.manifest_sha256 {
        return Err(VerificationError::HashMismatch(PathBuf::from("manifest.json")).into());
    }

    Ok((
        verified_capsule.manifest.files.len() + 1,
        staged.verification,
    ))
}

fn validate_destination(path: &Path) -> Result<(), IntegrationError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(IntegrationError::UnsafeDestination(path.to_path_buf()));
    }
    Ok(())
}

fn safe_capsule_relative_path(path: &str) -> Result<PathBuf, IntegrationError> {
    let path = PathBuf::from(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(IntegrationError::UnsafeDestination(path));
    }
    Ok(path)
}

fn ensure_target_parent(
    target_root: &Path,
    relative_parent: Option<&Path>,
) -> Result<(), IntegrationError> {
    let Some(relative_parent) = relative_parent else {
        return Ok(());
    };

    let mut current = target_root.to_path_buf();
    for component in relative_parent.components() {
        let Component::Normal(name) = component else {
            return Err(IntegrationError::UnsafeDestination(
                relative_parent.to_path_buf(),
            ));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(IntegrationError::UnsafeTargetComponent(current));
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                fs::create_dir(&current)?;
            }
            Err(error) => return Err(error.into()),
        }

        let canonical = current.canonicalize()?;
        if !canonical.starts_with(target_root) {
            return Err(IntegrationError::UnsafeTargetComponent(current));
        }
    }

    Ok(())
}

fn integration_readme(plan: &CompatibilityPlan) -> String {
    let strategy = format!("{:?}", plan.strategy);
    let level = format!("{:?}", plan.level);
    let mut content = format!(
        "# Software Composer integration: {}\n\n- Compatibility: {}\n- Strategy: {}\n- Capsule ID: {}\n- Verified capsule files: {}\n\n",
        plan.capability_id,
        level,
        strategy,
        plan.capsule_verification.capsule_id,
        plan.capsule_verification.verified_files,
    );

    content.push_str("## Staged contents\n\n");
    content.push_str("- capsule/ contains verified source, dependency manifests, license evidence, and the original capsule manifest.\n");
    content.push_str("- integration-plan.json contains the target compatibility assessment.\n");
    content.push_str("- adapter-contract.json is a draft contract, not generated adapter code.\n");
    content
        .push_str("- capsule-verification.json records the source capsule hash verification.\n\n");

    content.push_str("## Required review\n\n");
    content.push_str("1. Review every warning and unresolved import in integration-plan.json.\n");
    content.push_str("2. Resolve package dependencies in the target's native package manager. Software Composer does not install packages.\n");
    content.push_str("3. Review adapter-contract.json; map public interfaces, types, errors, configuration, and lifecycle manually.\n");
    content.push_str("4. Wire the feature into the target application's composition root.\n");
    content.push_str(
        "5. Add and run contract, integration, and regression tests in an approved sandbox.\n\n",
    );

    content.push_str("## Execution policy\n\n");
    content.push_str("This staging operation did not modify existing target application files, run build scripts, install dependencies, execute tests, or execute capsule source. It created this new directory only. The capsule is hash-verified but is not authenticated by a digital signature and is not certified as buildable or legally cleared.\n");
    content
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use composer_intelligence::{CapsuleOptions, analyze_repository, create_feature_capsule};
    use uuid::Uuid;

    use super::*;

    fn fixture() -> (PathBuf, PathBuf, PathBuf) {
        let workspace =
            std::env::temp_dir().join(format!("composer-integration-{}", Uuid::new_v4()));
        let source = workspace.join("source");
        let target = workspace.join("target");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(target.join("src")).unwrap();
        fs::write(source.join("Cargo.toml"), "[package]\nname = \"source_feature\"\nversion = \"0.1.0\"\nlicense = \"MIT OR Apache-2.0\"\n[dependencies]\nserde = \"1\"\n").unwrap();
        fs::write(source.join("LICENSE-MIT"), "MIT License\n").unwrap();
        fs::write(source.join("src/feature.rs"), "pub fn feature_entry() {}\n").unwrap();
        fs::write(target.join("Cargo.toml"), "[package]\nname = \"target_app\"\nversion = \"0.1.0\"\n[dependencies]\nserde = \"1\"\n").unwrap();
        fs::write(target.join("src/main.rs"), "fn main() {}\n").unwrap();

        let graph = analyze_repository(&source).unwrap();
        let seed = graph
            .symbols
            .iter()
            .find(|symbol| symbol.name == "feature_entry")
            .unwrap();
        let capsule_path = workspace.join("capsule");
        create_feature_capsule(
            &graph,
            &CapsuleOptions {
                capability_id: "feature-entry".to_owned(),
                description: "Feature entry".to_owned(),
                seed_symbol_ids: vec![seed.id.clone()],
                output_dir: capsule_path.clone(),
            },
        )
        .unwrap();

        (workspace, capsule_path, target)
    }

    #[test]
    fn applies_to_a_new_target_namespace_and_reverifies_the_copy() {
        let (workspace, capsule, target) = fixture();
        let destination = PathBuf::from(".composer/integrations/feature-entry");
        let result = apply_integration(&IntegrationOptions {
            capsule_root: capsule,
            target_root: target.clone(),
            destination: Some(destination.clone()),
        })
        .unwrap();

        assert_eq!(result.copied_capsule_files, 4);
        assert_eq!(result.staged_verification.source_files, 1);
        assert!(result.plan.dependencies.iter().any(|item| {
            item.dependency.name == "serde"
                && item.status == crate::RequirementStatus::AlreadyPresent
        }));
        assert_eq!(result.plan.level, crate::CompatibilityLevel::Compatible);
        assert!(!result.build_or_test_commands_executed);
        assert!(result.destination.join("capsule/manifest.json").is_file());
        assert!(result.destination.join("integration-plan.json").is_file());
        assert!(target.join("src/main.rs").is_file());
        assert!(!target.join("src/feature.rs").exists());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn refuses_to_overwrite_an_existing_integration() {
        let (workspace, capsule, target) = fixture();
        let destination = target.join(".composer/integrations/feature-entry");
        fs::create_dir_all(&destination).unwrap();
        let destination = destination.canonicalize().unwrap();
        let result = apply_integration(&IntegrationOptions {
            capsule_root: capsule,
            target_root: target.clone(),
            destination: Some(PathBuf::from(".composer/integrations/feature-entry")),
        });

        assert!(
            matches!(&result, Err(IntegrationError::DestinationExists(path)) if *path == destination),
            "unexpected overwrite result: {result:?}; expected existing path: {}",
            destination.display()
        );
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn rejects_parent_traversal_in_destination() {
        let (workspace, capsule, target) = fixture();
        let result = apply_integration(&IntegrationOptions {
            capsule_root: capsule,
            target_root: target,
            destination: Some(PathBuf::from("../escape")),
        });

        assert!(matches!(
            result,
            Err(IntegrationError::UnsafeDestination(_))
        ));
        let _ = fs::remove_dir_all(workspace);
    }
}
