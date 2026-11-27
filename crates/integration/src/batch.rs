//! Bounded multi-capsule Rust composition using separately verified steps.
//! Every step is independently compiled and tested in the existing offline
//! Docker runner. The requested destination is never published on failure.
use std::{collections::HashSet, fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    VerifiedRustError, VerifiedRustOptions, VerifiedRustResult, compose_verified_rust,
    verify_capsule,
};

const MAX_CAPSULES: usize = 5;

#[derive(Debug, Clone)]
pub struct BatchRustOptions {
    pub capsule_roots: Vec<PathBuf>,
    pub target_root: PathBuf,
    pub output_root: PathBuf,
    pub license_reviewed: bool,
    pub registry_cache: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRustResult {
    pub output_root: PathBuf,
    pub integrations_verified: usize,
    pub steps: Vec<BatchStep>,
    pub evidence_path: PathBuf,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchStep {
    pub capability_id: String,
    pub capsule_id: String,
    pub dependencies_added: Vec<String>,
    pub duration_seconds: f64,
    pub individual_evidence: serde_json::Value,
}

#[derive(Debug, Error)]
pub enum BatchRustError {
    #[error("source/license review must be explicitly asserted")]
    ReviewRequired,
    #[error("2 to {MAX_CAPSULES} distinct Feature Capsules are required")]
    CapsuleCount,
    #[error("duplicate capability ID: {0}")]
    DuplicateCapability(String),
    #[error("output already exists: {0}")]
    OutputExists(PathBuf),
    #[error("invalid output: {0}")]
    InvalidOutput(String),
    #[error("capsule verification failed: {0}")]
    Capsule(#[from] crate::VerificationError),
    #[error("step {step} ({capability_id}) failed; nothing published: {source}")]
    Step {
        step: usize,
        capability_id: String,
        #[source]
        source: VerifiedRustError,
    },
    #[error("file operation failed: {0}")]
    Filesystem(#[from] std::io::Error),
    #[error("JSON evidence failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// Compose a small set of previously reviewed Rust capsules from potentially
/// different repositories. Tests run after EACH change; if step N fails, no
/// requested output is published and all previous disposable copies are
/// cleaned up. Neither the source capsules nor the target are changed.
pub fn compose_verified_rust_batch(
    options: &BatchRustOptions,
) -> Result<BatchRustResult, BatchRustError> {
    validate_batch(options)?;
    let parent = options
        .output_root
        .parent()
        .ok_or_else(|| BatchRustError::InvalidOutput("output must have an existing parent".into()))?
        .canonicalize()?;
    let filename = options
        .output_root
        .file_name()
        .ok_or_else(|| BatchRustError::InvalidOutput("output must have a directory name".into()))?;
    if !matches!(
        options.output_root.components().next_back(),
        Some(std::path::Component::Normal(_))
    ) {
        return Err(BatchRustError::InvalidOutput(
            "output path must end with a directory name".into(),
        ));
    }
    let final_output = parent.join(filename);
    if final_output.exists() {
        return Err(BatchRustError::OutputExists(final_output));
    }
    let target = options.target_root.canonicalize()?;
    if target.starts_with(&final_output) || final_output.starts_with(&target) {
        return Err(BatchRustError::InvalidOutput(
            "output cannot overlap target".into(),
        ));
    }

    let nonce = Uuid::new_v4();
    let mut intermediates = Vec::<PathBuf>::new();
    let result = (|| {
        let mut steps = Vec::new();
        let mut current_input = target;
        for (index, capsule_root) in options.capsule_roots.iter().enumerate() {
            let verified = verify_capsule(capsule_root)?;
            let next_output = parent.join(format!(".composer-batch-{nonce}-{}.tmp", index + 1));
            // Each step owns a new target copy and runs independent Docker tests.
            let step = compose_verified_rust(&VerifiedRustOptions {
                capsule_root: capsule_root.clone(),
                target_root: current_input.clone(),
                output_root: next_output.clone(),
                license_reviewed: true,
                registry_cache: options.registry_cache.clone(),
                api_aliases: Vec::new(),
                api_signatures: Vec::new(),
            })
            .map_err(|source| BatchRustError::Step {
                step: index + 1,
                capability_id: verified.manifest.capability_id.clone(),
                source,
            })?;
            intermediates.push(next_output.clone());
            steps.push(snapshot_step(&verified.manifest.capability_id, &step)?);
            current_input = next_output;
        }
        // Persist all individual step evidence, not just the final test.
        let report_name = "composer-batch-evidence.json";
        fs::write(
            current_input.join(report_name),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": 1,
                "kind": "bounded_rust_batch",
                "integrations_verified": steps.len(),
                "published_only_after_every_step_passes": true,
                "steps": steps,
            }))?,
        )?;
        fs::rename(&current_input, &final_output)?;
        intermediates.pop(); // Last path was renamed into the final output.
        Ok(BatchRustResult {
            output_root: final_output.clone(),
            integrations_verified: steps.len(),
            steps,
            evidence_path: final_output.join(report_name),
            status: "verified_all_stages".into(),
        })
    })();
    // This cleanup always runs, including on any compilation/test failure.
    for directory in intermediates {
        let _ = fs::remove_dir_all(directory);
    }
    result
}

fn snapshot_step(id: &str, result: &VerifiedRustResult) -> Result<BatchStep, BatchRustError> {
    let bytes = fs::read(&result.evidence_path)?;
    let individual_evidence = serde_json::from_slice(&bytes)?;
    Ok(BatchStep {
        capability_id: id.to_owned(),
        capsule_id: result.capsule_id.clone(),
        dependencies_added: result.new_dependencies.clone(),
        duration_seconds: result.elapsed_seconds,
        individual_evidence,
    })
}

fn validate_batch(options: &BatchRustOptions) -> Result<(), BatchRustError> {
    if !options.license_reviewed {
        return Err(BatchRustError::ReviewRequired);
    }
    if !(2..=MAX_CAPSULES).contains(&options.capsule_roots.len()) {
        return Err(BatchRustError::CapsuleCount);
    }
    if options.output_root.exists() {
        return Err(BatchRustError::OutputExists(options.output_root.clone()));
    }
    let mut capability_ids = HashSet::new();
    for capsule in &options.capsule_roots {
        let verified = verify_capsule(capsule)?;
        if !capability_ids.insert(verified.manifest.capability_id.clone()) {
            return Err(BatchRustError::DuplicateCapability(
                verified.manifest.capability_id.clone(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_missing_review_before_touching_sources() {
        let options = BatchRustOptions {
            capsule_roots: vec![PathBuf::from("does-not-exist"); 2],
            target_root: PathBuf::from("not-a-target"),
            output_root: PathBuf::from("not-an-output"),
            license_reviewed: false,
            registry_cache: None,
        };
        assert!(matches!(
            compose_verified_rust_batch(&options),
            Err(BatchRustError::ReviewRequired)
        ));
    }
    #[test]
    fn rejects_empty_and_oversized_batches() {
        for count in [0, 1, 6] {
            let options = BatchRustOptions {
                capsule_roots: vec![PathBuf::from("no-capsule"); count],
                target_root: PathBuf::from("no-target"),
                output_root: PathBuf::from("no-output"),
                license_reviewed: true,
                registry_cache: None,
            };
            assert!(matches!(
                compose_verified_rust_batch(&options),
                Err(BatchRustError::CapsuleCount)
            ));
        }
    }
}
