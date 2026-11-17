use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use composer_intelligence::{
    AnalysisError, RepositoryGraph, SourceLanguage, SymbolKind, analyze_repository,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    dependencies::{
        DependencyEcosystem, DependencyError, DependencyInventory, DependencyRequirement,
        RequirementStatus, discover_manifest_paths, inventory_manifests,
    },
    verify::{CapsuleVerification, VerificationError, VerifiedCapsule, verify_capsule},
};

const MAX_CONTRACT_SYMBOLS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityLevel {
    Compatible,
    RequiresAdapter,
    ReviewRequired,
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStrategy {
    NativeModuleCandidate,
    IsolatedComponent,
    CrossLanguageAdapterRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityFinding {
    pub code: String,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetProfile {
    pub root: PathBuf,
    pub source_files: usize,
    pub symbols: usize,
    pub languages: Vec<SourceLanguage>,
    pub primary_language: Option<SourceLanguage>,
    pub package_managers: Vec<DependencyEcosystem>,
    pub candidate_test_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolCollision {
    pub name: String,
    pub capsule_kind: SymbolKind,
    pub capsule_file: PathBuf,
    pub target_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestEvidence {
    pub location: String,
    pub path: PathBuf,
    pub detection: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyAssessment {
    pub dependency: DependencyRequirement,
    pub status: RequirementStatus,
    pub target_requirement: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub source_file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterContract {
    pub schema_version: u32,
    pub capability_id: String,
    pub source_languages: Vec<SourceLanguage>,
    pub target_languages: Vec<SourceLanguage>,
    pub strategy: IntegrationStrategy,
    pub candidate_symbols: Vec<ContractSymbol>,
    pub requires_human_mapping: bool,
    pub unresolved_contract_questions: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityPlan {
    pub schema_version: u32,
    pub capability_id: String,
    pub capsule_root: PathBuf,
    pub target_root: PathBuf,
    pub capsule_verification: CapsuleVerification,
    pub level: CompatibilityLevel,
    pub strategy: IntegrationStrategy,
    pub recommended_destination: PathBuf,
    pub target: TargetProfile,
    pub source_languages: Vec<SourceLanguage>,
    pub dependencies: Vec<DependencyAssessment>,
    pub symbol_collisions: Vec<SymbolCollision>,
    pub test_evidence: Vec<TestEvidence>,
    pub findings: Vec<CompatibilityFinding>,
    pub adapter_contract: AdapterContract,
    pub source_license_review_required: bool,
}

#[derive(Debug, Error)]
pub enum CompatibilityError {
    #[error("capsule verification failed: {0}")]
    CapsuleVerification(#[from] VerificationError),
    #[error("repository analysis failed: {0}")]
    Analysis(#[from] AnalysisError),
    #[error("package manifest inventory failed: {0}")]
    Dependencies(#[from] DependencyError),
    #[error("target repository does not exist: {0}")]
    MissingTarget(PathBuf),
    #[error("target repository is not a directory: {0}")]
    TargetNotDirectory(PathBuf),
    #[error("filesystem operation failed: {0}")]
    Filesystem(#[from] std::io::Error),
}

pub fn plan_integration(
    capsule_root: &Path,
    target_root: &Path,
) -> Result<CompatibilityPlan, CompatibilityError> {
    if !target_root.exists() {
        return Err(CompatibilityError::MissingTarget(target_root.to_path_buf()));
    }
    if !target_root.is_dir() {
        return Err(CompatibilityError::TargetNotDirectory(
            target_root.to_path_buf(),
        ));
    }

    let verified = verify_capsule(capsule_root)?;
    let target_root = target_root.canonicalize()?;
    let target_graph = analyze_repository(&target_root)?;
    let capsule_source_root = verified.root.join("source");
    let capsule_graph = analyze_repository(&capsule_source_root)?;

    let capsule_manifest_paths = verified
        .manifest
        .files
        .iter()
        .filter(|file| file.file_kind == "dependency_manifest")
        .map(|file| PathBuf::from(&file.capsule_path))
        .collect::<Vec<_>>();
    let capsule_inventory = inventory_manifests(&verified.root, &capsule_manifest_paths)?;

    let target_manifest_paths = discover_manifest_paths(&target_root)?;
    let target_inventory = inventory_manifests(&target_root, &target_manifest_paths)?;

    let source_languages = capsule_graph.summary.languages.clone();
    let target_languages = target_graph.summary.languages.clone();
    let strategy = choose_strategy(&source_languages, &target_languages);
    let symbol_collisions = find_symbol_collisions(&capsule_graph, &target_graph);
    let dependency_assessments = assess_dependencies(&capsule_inventory, &target_inventory);

    let mut findings = Vec::new();
    add_compatibility_findings(
        CompatibilityFindingContext {
            verified: &verified,
            capsule_graph: &capsule_graph,
            target_graph: &target_graph,
            capsule_inventory: &capsule_inventory,
            target_inventory: &target_inventory,
            strategy: &strategy,
            collisions: &symbol_collisions,
            dependencies: &dependency_assessments,
        },
        &mut findings,
    );

    let requires_review = !symbol_collisions.is_empty()
        || dependencies_require_review(&dependency_assessments)
        || !verified.manifest.external_or_unresolved_imports.is_empty()
        || capsule_graph
            .files
            .iter()
            .any(|file| file.has_syntax_errors);
    let level = derive_compatibility_level(
        !target_graph.files.is_empty(),
        requires_review,
        strategy == IntegrationStrategy::CrossLanguageAdapterRequired,
    );

    let target_profile = target_profile(&target_root, &target_graph, &target_manifest_paths);
    let test_evidence = collect_test_evidence(&capsule_graph, &target_graph);
    let adapter_contract = create_adapter_contract(
        &verified.manifest.capability_id,
        &source_languages,
        &target_languages,
        strategy,
        &capsule_graph,
        !symbol_collisions.is_empty(),
    );

    Ok(CompatibilityPlan {
        schema_version: 1,
        capability_id: verified.manifest.capability_id.clone(),
        capsule_root: verified.root,
        target_root: target_root.clone(),
        capsule_verification: verified.verification,
        level,
        strategy,
        recommended_destination: PathBuf::from(".composer")
            .join("integrations")
            .join(&verified.manifest.capability_id),
        target: target_profile,
        source_languages,
        dependencies: dependency_assessments,
        symbol_collisions,
        test_evidence,
        findings,
        adapter_contract,
        source_license_review_required: verified.manifest.license_review_required,
    })
}

fn choose_strategy(
    source_languages: &[SourceLanguage],
    target_languages: &[SourceLanguage],
) -> IntegrationStrategy {
    if !source_languages.is_empty()
        && source_languages
            .iter()
            .all(|language| target_languages.contains(language))
    {
        IntegrationStrategy::NativeModuleCandidate
    } else if source_languages
        .iter()
        .any(|language| target_languages.contains(language))
    {
        IntegrationStrategy::IsolatedComponent
    } else {
        IntegrationStrategy::CrossLanguageAdapterRequired
    }
}

fn find_symbol_collisions(
    capsule_graph: &RepositoryGraph,
    target_graph: &RepositoryGraph,
) -> Vec<SymbolCollision> {
    let mut target_by_name: HashMap<String, BTreeSet<PathBuf>> = HashMap::new();
    for symbol in &target_graph.symbols {
        target_by_name
            .entry(normalize_symbol_name(&symbol.name))
            .or_default()
            .insert(symbol.file_path.clone());
    }

    let mut collisions = BTreeMap::new();
    for symbol in &capsule_graph.symbols {
        let key = normalize_symbol_name(&symbol.name);
        if let Some(target_files) = target_by_name.get(&key) {
            let entry = collisions
                .entry(key.clone())
                .or_insert_with(|| SymbolCollision {
                    name: symbol.name.clone(),
                    capsule_kind: symbol.kind,
                    capsule_file: symbol.file_path.clone(),
                    target_files: target_files.iter().cloned().collect(),
                });
            for file in target_files {
                if !entry.target_files.contains(file) {
                    entry.target_files.push(file.clone());
                }
            }
        }
    }

    collisions.into_values().collect()
}

fn normalize_symbol_name(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn assess_dependencies(
    capsule: &DependencyInventory,
    target: &DependencyInventory,
) -> Vec<DependencyAssessment> {
    let target_dependencies = &target.dependencies;
    capsule
        .dependencies
        .iter()
        .map(|dependency| {
            let matches = target_dependencies
                .iter()
                .filter(|candidate| {
                    candidate.ecosystem == dependency.ecosystem
                        && candidate.name.eq_ignore_ascii_case(&dependency.name)
                })
                .collect::<Vec<_>>();

            let (status, target_requirement) = match matches.as_slice() {
                [] => (RequirementStatus::NotDeclaredInTarget, None),
                [candidate] if candidate.requirement == dependency.requirement => (
                    RequirementStatus::AlreadyPresent,
                    Some(candidate.requirement.clone()),
                ),
                [candidate] => (
                    RequirementStatus::DifferentConstraint,
                    Some(candidate.requirement.clone()),
                ),
                candidates => {
                    let value = candidates
                        .iter()
                        .map(|candidate| candidate.requirement.as_str())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join(" | ");
                    (RequirementStatus::DifferentConstraint, Some(value))
                }
            };

            DependencyAssessment {
                dependency: dependency.clone(),
                status,
                target_requirement,
            }
        })
        .collect()
}

struct CompatibilityFindingContext<'a> {
    verified: &'a VerifiedCapsule,
    capsule_graph: &'a RepositoryGraph,
    target_graph: &'a RepositoryGraph,
    capsule_inventory: &'a DependencyInventory,
    target_inventory: &'a DependencyInventory,
    strategy: &'a IntegrationStrategy,
    collisions: &'a [SymbolCollision],
    dependencies: &'a [DependencyAssessment],
}

fn add_compatibility_findings(
    context: CompatibilityFindingContext<'_>,
    findings: &mut Vec<CompatibilityFinding>,
) {
    let CompatibilityFindingContext {
        verified,
        capsule_graph,
        target_graph,
        capsule_inventory,
        target_inventory,
        strategy,
        collisions,
        dependencies,
    } = context;
    findings.push(CompatibilityFinding {
        code: "CAPSULE_HASHES_VERIFIED".to_owned(),
        severity: "info".to_owned(),
        message: format!(
            "Verified {} capsule files against the capsule manifest.",
            verified.verification.verified_files
        ),
    });

    findings.push(CompatibilityFinding {
        code: "TARGET_ANALYZED".to_owned(),
        severity: "info".to_owned(),
        message: format!(
            "Analyzed {} target source files and {} symbols.",
            target_graph.summary.files_analyzed, target_graph.summary.symbols_found
        ),
    });

    if *strategy == IntegrationStrategy::NativeModuleCandidate {
        findings.push(CompatibilityFinding {
            code: "LANGUAGE_ALIGNMENT".to_owned(),
            severity: "info".to_owned(),
            message: "All capsule source languages are already present in the target.".to_owned(),
        });
    } else if *strategy == IntegrationStrategy::IsolatedComponent {
        findings.push(CompatibilityFinding {
            code: "PARTIAL_LANGUAGE_ALIGNMENT".to_owned(),
            severity: "warning".to_owned(),
            message: "The target supports some, but not all, capsule source languages. Keep the feature isolated until cross-language edges are mapped.".to_owned(),
        });
    } else {
        findings.push(CompatibilityFinding {
            code: "CROSS_LANGUAGE_ADAPTER_REQUIRED".to_owned(),
            severity: "warning".to_owned(),
            message: "No capsule source language matches the target. An explicit bridge or service contract is required; no adapter code is generated automatically.".to_owned(),
        });
    }

    if !collisions.is_empty() {
        findings.push(CompatibilityFinding {
            code: "SYMBOL_NAME_COLLISIONS".to_owned(),
            severity: "warning".to_owned(),
            message: format!(
                "{} candidate symbol names also occur in the target. These may be equivalent implementations or unrelated names; review before wiring calls.",
                collisions.len()
            ),
        });
    }

    let missing = dependencies
        .iter()
        .filter(|assessment| assessment.status != RequirementStatus::AlreadyPresent)
        .count();
    if missing > 0 {
        findings.push(CompatibilityFinding {
            code: "DEPENDENCIES_NEED_REVIEW".to_owned(),
            severity: "warning".to_owned(),
            message: format!(
                "{missing} package requirement(s) are absent from or differ in the target manifest set."
            ),
        });
    }

    if !capsule_inventory.license_declarations.is_empty() {
        let invalid = capsule_inventory
            .license_declarations
            .iter()
            .filter(|declaration| !declaration.syntax_valid)
            .count();
        findings.push(CompatibilityFinding {
            code: "DECLARED_LICENSE_EXPRESSIONS".to_owned(),
            severity: "warning".to_owned(),
            message: format!(
                "Found {} package license declaration(s); {invalid} failed syntax-only SPDX expression parsing. Identifier validity and legal compatibility were not evaluated.",
                capsule_inventory.license_declarations.len()
            ),
        });
    } else {
        findings.push(CompatibilityFinding {
            code: "NO_PACKAGE_LICENSE_DECLARATION".to_owned(),
            severity: "warning".to_owned(),
            message: "No package-level SPDX license expression was found in the copied manifests."
                .to_owned(),
        });
    }

    if verified.manifest.license_review_required
        || verified.manifest.license_evidence_files.is_empty()
    {
        findings.push(CompatibilityFinding {
            code: "LICENSE_REVIEW_REQUIRED".to_owned(),
            severity: "warning".to_owned(),
            message: "License evidence is not a legal clearance. Review upstream license texts, notices, and file-level obligations before redistribution.".to_owned(),
        });
    }

    if !verified.manifest.external_or_unresolved_imports.is_empty() {
        findings.push(CompatibilityFinding {
            code: "UNRESOLVED_SOURCE_IMPORTS".to_owned(),
            severity: "warning".to_owned(),
            message: format!(
                "{} source import edge(s) remain unresolved in the capsule.",
                verified.manifest.external_or_unresolved_imports.len()
            ),
        });
    }

    if !capsule_inventory.warnings.is_empty() || !target_inventory.warnings.is_empty() {
        findings.push(CompatibilityFinding {
            code: "MANIFEST_PARSE_WARNINGS".to_owned(),
            severity: "warning".to_owned(),
            message: format!(
                "{} capsule and target manifest parsing warning(s) were recorded.",
                capsule_inventory.warnings.len() + target_inventory.warnings.len()
            ),
        });
    }

    if capsule_graph
        .files
        .iter()
        .any(|file| file.has_syntax_errors)
    {
        findings.push(CompatibilityFinding {
            code: "CAPSULE_SYNTAX_ERRORS".to_owned(),
            severity: "warning".to_owned(),
            message: "One or more capsule source files contain Tree-sitter syntax errors."
                .to_owned(),
        });
    }
}

fn target_profile(
    root: &Path,
    graph: &RepositoryGraph,
    manifest_paths: &[PathBuf],
) -> TargetProfile {
    let mut package_managers = BTreeSet::new();
    for path in manifest_paths {
        match path.file_name().and_then(|name| name.to_str()) {
            Some("Cargo.toml") => {
                package_managers.insert(DependencyEcosystem::Cargo);
            }
            Some("package.json") => {
                package_managers.insert(DependencyEcosystem::Npm);
            }
            Some("pyproject.toml" | "requirements.txt") => {
                package_managers.insert(DependencyEcosystem::Python);
            }
            Some("go.mod") => {
                package_managers.insert(DependencyEcosystem::Go);
            }
            _ => {}
        }
    }

    let mut counts = BTreeMap::new();
    for file in &graph.files {
        *counts.entry(file.language).or_insert(0usize) += 1;
    }
    let primary_language = counts
        .iter()
        .max_by_key(|(_, count)| **count)
        .map(|(language, _)| *language);

    let candidate_test_files = graph
        .files
        .iter()
        .filter(|file| looks_like_test_file(&file.path))
        .map(|file| file.path.clone())
        .collect();

    TargetProfile {
        root: root.to_path_buf(),
        source_files: graph.summary.files_analyzed,
        symbols: graph.summary.symbols_found,
        languages: graph.summary.languages.clone(),
        primary_language,
        package_managers: package_managers.into_iter().collect(),
        candidate_test_files,
    }
}

fn collect_test_evidence(
    capsule_graph: &RepositoryGraph,
    target_graph: &RepositoryGraph,
) -> Vec<TestEvidence> {
    let mut evidence = Vec::new();
    for file in &capsule_graph.files {
        if looks_like_test_file(&file.path) {
            evidence.push(TestEvidence {
                location: "capsule".to_owned(),
                path: file.path.clone(),
                detection: "test_path_convention".to_owned(),
            });
        }
    }
    for file in &target_graph.files {
        if looks_like_test_file(&file.path) {
            evidence.push(TestEvidence {
                location: "target".to_owned(),
                path: file.path.clone(),
                detection: "test_path_convention".to_owned(),
            });
        }
    }
    evidence
}

fn looks_like_test_file(path: &Path) -> bool {
    let is_tests_directory = path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("tests")
    });
    if is_tests_directory {
        return true;
    }

    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = file_name.to_ascii_lowercase();
    lower.starts_with("test_")
        || lower.ends_with("_test.go")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower == "tests.rs"
}

fn create_adapter_contract(
    capability_id: &str,
    source_languages: &[SourceLanguage],
    target_languages: &[SourceLanguage],
    strategy: IntegrationStrategy,
    capsule_graph: &RepositoryGraph,
    has_symbol_conflicts: bool,
) -> AdapterContract {
    let candidate_symbols = capsule_graph
        .symbols
        .iter()
        .filter(|symbol| {
            matches!(
                symbol.kind,
                SymbolKind::Function
                    | SymbolKind::Class
                    | SymbolKind::Struct
                    | SymbolKind::Enum
                    | SymbolKind::Trait
                    | SymbolKind::Interface
            )
        })
        .take(MAX_CONTRACT_SYMBOLS)
        .map(|symbol| ContractSymbol {
            name: symbol.name.clone(),
            kind: symbol.kind,
            source_file: symbol.file_path.clone(),
            start_line: symbol.start_line,
            end_line: symbol.end_line,
        })
        .collect();

    let mut questions = vec![
        "Confirm the capsule's public API and visibility; Tree-sitter declaration discovery does not prove that a symbol is externally callable.".to_owned(),
        "Map source types, error behavior, configuration, and lifecycle to the target architecture.".to_owned(),
        "Review runtime services and external dependencies that are not expressible as package requirements.".to_owned(),
        "Select and run target-specific contract and regression tests after wiring the adapter.".to_owned(),
    ];
    if has_symbol_conflicts {
        questions.push(
            "Resolve candidate symbol-name collisions with existing target declarations."
                .to_owned(),
        );
    }
    if strategy == IntegrationStrategy::CrossLanguageAdapterRequired {
        questions.push("Define a process, RPC, or data-contract boundary; direct source transplantation is not compatible.".to_owned());
    }

    AdapterContract {
        schema_version: 1,
        capability_id: capability_id.to_owned(),
        source_languages: source_languages.to_vec(),
        target_languages: target_languages.to_vec(),
        strategy,
        candidate_symbols,
        requires_human_mapping: true,
        unresolved_contract_questions: questions,
        status: "draft_requires_review".to_owned(),
    }
}

fn dependencies_require_review(dependencies: &[DependencyAssessment]) -> bool {
    dependencies
        .iter()
        .any(|assessment| assessment.status != RequirementStatus::AlreadyPresent)
}

fn derive_compatibility_level(
    target_has_source: bool,
    requires_review: bool,
    requires_adapter: bool,
) -> CompatibilityLevel {
    if !target_has_source {
        CompatibilityLevel::Incompatible
    } else if requires_review {
        CompatibilityLevel::ReviewRequired
    } else if requires_adapter {
        CompatibilityLevel::RequiresAdapter
    } else {
        CompatibilityLevel::Compatible
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dependencies::DependencyKind;

    fn dependency_assessment(status: RequirementStatus) -> DependencyAssessment {
        DependencyAssessment {
            dependency: DependencyRequirement {
                ecosystem: DependencyEcosystem::Cargo,
                name: "serde".to_owned(),
                requirement: "version=1".to_owned(),
                kind: DependencyKind::Runtime,
                source_manifest: PathBuf::from("Cargo.toml"),
            },
            status,
            target_requirement: None,
        }
    }

    #[test]
    fn missing_or_different_direct_dependencies_require_review() {
        assert!(dependencies_require_review(&[dependency_assessment(
            RequirementStatus::NotDeclaredInTarget
        )]));
        assert!(dependencies_require_review(&[dependency_assessment(
            RequirementStatus::DifferentConstraint
        )]));
        assert!(!dependencies_require_review(&[dependency_assessment(
            RequirementStatus::AlreadyPresent
        )]));
    }

    #[test]
    fn review_findings_take_precedence_over_adapter_strategy() {
        assert_eq!(
            derive_compatibility_level(true, true, true),
            CompatibilityLevel::ReviewRequired
        );
        assert_eq!(
            derive_compatibility_level(true, false, true),
            CompatibilityLevel::RequiresAdapter
        );
        assert_eq!(
            derive_compatibility_level(true, false, false),
            CompatibilityLevel::Compatible
        );
        assert_eq!(
            derive_compatibility_level(false, false, false),
            CompatibilityLevel::Incompatible
        );
    }
}
