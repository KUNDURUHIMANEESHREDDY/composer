use std::collections::{BTreeSet, HashMap, VecDeque};

use thiserror::Error;

use crate::{
    RepositoryGraph,
    model::{
        ImportDependency, ImportResolution, Symbol, SymbolKind, SymbolReference, SymbolResolution,
    },
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FeatureCandidate {
    pub symbol_id: String,
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: std::path::PathBuf,
    /// Lexical relevance score from 0 to 100. This is a ranking hint, not proof.
    pub score: u8,
    pub matched_terms: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FeatureBoundary {
    pub capability_id: String,
    pub description: String,
    pub boundary_strategy: String,
    pub seed_symbol_ids: Vec<String>,
    pub included_files: Vec<std::path::PathBuf>,
    pub included_symbol_ids: Vec<String>,
    pub unresolved_imports: Vec<ImportDependency>,
    pub unfollowed_symbol_references: Vec<SymbolReference>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BoundaryError {
    #[error("capability id must use lowercase kebab-case")]
    InvalidCapabilityId,
    #[error("at least one seed symbol is required")]
    NoSeedSymbols,
    #[error("seed symbol '{0}' does not exist in the repository graph")]
    UnknownSeedSymbol(String),
}

pub fn recommend_feature_seeds(
    graph: &RepositoryGraph,
    capability_id: &str,
    description: &str,
    limit: usize,
) -> Vec<FeatureCandidate> {
    let terms = capability_terms(capability_id, description);
    if terms.is_empty() || limit == 0 {
        return Vec::new();
    }

    let candidates = graph
        .symbols
        .iter()
        .filter_map(|symbol| score_symbol(symbol, &terms))
        .filter(|candidate| candidate.score > 0)
        .collect::<Vec<_>>();

    let mut candidates = candidates;
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.file_path.cmp(&right.file_path))
            .then_with(|| left.name.cmp(&right.name))
    });
    candidates.truncate(limit);
    candidates
}

pub fn analyze_feature_boundary(
    graph: &RepositoryGraph,
    capability_id: &str,
    description: &str,
    seed_symbol_ids: &[String],
) -> Result<FeatureBoundary, BoundaryError> {
    if !is_valid_capability_id(capability_id) {
        return Err(BoundaryError::InvalidCapabilityId);
    }
    if seed_symbol_ids.is_empty() {
        return Err(BoundaryError::NoSeedSymbols);
    }

    let symbols_by_id: HashMap<&str, &Symbol> = graph
        .symbols
        .iter()
        .map(|symbol| (symbol.id.as_str(), symbol))
        .collect();

    for seed in seed_symbol_ids {
        if !symbols_by_id.contains_key(seed.as_str()) {
            return Err(BoundaryError::UnknownSeedSymbol(seed.clone()));
        }
    }

    let mut included_files = BTreeSet::new();
    let mut included_symbols = BTreeSet::new();
    let mut pending_files = VecDeque::new();

    for seed in seed_symbol_ids {
        let symbol = symbols_by_id[seed.as_str()];
        included_symbols.insert(seed.clone());
        if included_files.insert(symbol.file_path.clone()) {
            pending_files.push_back(symbol.file_path.clone());
        }
    }

    let mut unresolved_imports = Vec::new();
    let mut unfollowed_references = Vec::new();
    let mut handled_files = BTreeSet::new();

    while let Some(file_path) = pending_files.pop_front() {
        if !handled_files.insert(file_path.clone()) {
            continue;
        }

        for import in graph
            .imports
            .iter()
            .filter(|import| import.source_file == file_path)
        {
            match import.resolution {
                ImportResolution::Local => {
                    if let Some(target) = &import.resolved_file
                        && included_files.insert(target.clone())
                    {
                        pending_files.push_back(target.clone());
                    }
                }
                ImportResolution::UnresolvedOrExternal | ImportResolution::Ambiguous => {
                    unresolved_imports.push(import.clone());
                }
            }
        }

        // Call references are followed only when their target file is already
        // connected by a resolved import (or is this same file). Repository-wide
        // name matching alone is not sufficient evidence to pull in another file.
        for reference in graph
            .references
            .iter()
            .filter(|reference| reference.source_file == file_path)
        {
            match reference.resolution {
                SymbolResolution::UniqueNameMatch => {
                    if let Some(target_id) = &reference.resolved_symbol_id {
                        if let Some(target_symbol) = symbols_by_id.get(target_id.as_str()) {
                            if included_files.contains(&target_symbol.file_path) {
                                included_symbols.insert(target_id.clone());
                            } else {
                                unfollowed_references.push(reference.clone());
                            }
                        } else {
                            unfollowed_references.push(reference.clone());
                        }
                    } else {
                        unfollowed_references.push(reference.clone());
                    }
                }
                SymbolResolution::AmbiguousNameMatch | SymbolResolution::Unresolved => {
                    unfollowed_references.push(reference.clone());
                }
            }
        }
    }

    for symbol in &graph.symbols {
        if included_files.contains(&symbol.file_path) {
            included_symbols.insert(symbol.id.clone());
        }
    }

    let mut warnings = vec![
        "The boundary is a conservative static approximation; dynamic imports, reflection, macros, dependency injection, and runtime registration can add hidden dependencies.".to_owned(),
        "Call references are linked by unique symbol-name matches only and followed across files only when a resolved import already connects the target file; this is not compiler-verified.".to_owned(),
        "Whole source files are included. Unrelated declarations in the same file may be included to preserve local context.".to_owned(),
    ];

    if !unresolved_imports.is_empty() {
        warnings.push(format!(
            "{} import(s) remain unresolved or ambiguous; inspect them before treating the capsule as independently portable.",
            unresolved_imports.len()
        ));
    }
    if !unfollowed_references.is_empty() {
        warnings.push(format!(
            "{} call reference(s) were not followed into the capsule boundary; this includes unresolved/ambiguous matches and unique name matches without a connecting import edge.",
            unfollowed_references.len()
        ));
    }

    Ok(FeatureBoundary {
        capability_id: capability_id.to_owned(),
        description: description.to_owned(),
        boundary_strategy: "conservative_file_dependency_closure".to_owned(),
        seed_symbol_ids: seed_symbol_ids.to_vec(),
        included_files: included_files.into_iter().collect(),
        included_symbol_ids: included_symbols.into_iter().collect(),
        unresolved_imports,
        unfollowed_symbol_references: unfollowed_references,
        warnings,
    })
}

fn score_symbol(symbol: &Symbol, terms: &[String]) -> Option<FeatureCandidate> {
    let mut searchable = format!(
        "{} {} {}",
        symbol.name,
        symbol.file_path.to_string_lossy(),
        symbol.kind_name()
    );
    searchable = split_identifier_words(&searchable).join(" ");
    let searchable_terms: BTreeSet<String> = searchable
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();

    let matched_terms: Vec<String> = terms
        .iter()
        .filter(|term| searchable_terms.contains(term.as_str()))
        .cloned()
        .collect();

    if matched_terms.is_empty() {
        return None;
    }

    let score = ((matched_terms.len() as f32 / terms.len() as f32) * 100.0)
        .round()
        .clamp(1.0, 100.0) as u8;

    Some(FeatureCandidate {
        symbol_id: symbol.id.clone(),
        name: symbol.name.clone(),
        kind: symbol.kind,
        file_path: symbol.file_path.clone(),
        score,
        matched_terms,
    })
}

impl Symbol {
    fn kind_name(&self) -> &'static str {
        match self.kind {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Module => "module",
            SymbolKind::Constant => "constant",
            SymbolKind::Variable => "variable",
            SymbolKind::Macro => "macro",
        }
    }
}

fn capability_terms(capability_id: &str, description: &str) -> Vec<String> {
    let mut terms = BTreeSet::new();
    for word in split_identifier_words(&format!("{capability_id} {description}")) {
        let word = word.to_ascii_lowercase();
        if word.len() >= 3 && !is_stop_word(&word) {
            terms.insert(word);
        }
    }
    terms.into_iter().collect()
}

fn split_identifier_words(value: &str) -> Vec<String> {
    let mut normalized = String::with_capacity(value.len() + 8);
    let mut previous_is_lower_or_digit = false;

    for character in value.chars() {
        if character.is_ascii_uppercase() && previous_is_lower_or_digit {
            normalized.push(' ');
        }

        if character.is_ascii_alphanumeric() {
            normalized.push(character);
            previous_is_lower_or_digit =
                character.is_ascii_lowercase() || character.is_ascii_digit();
        } else {
            normalized.push(' ');
            previous_is_lower_or_digit = false;
        }
    }

    normalized.split_whitespace().map(str::to_owned).collect()
}

fn is_stop_word(value: &str) -> bool {
    matches!(
        value,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "into"
            | "using"
            | "system"
            | "support"
            | "manage"
            | "management"
            | "feature"
            | "application"
            | "project"
    )
}

fn is_valid_capability_id(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::{
        AnalysisSummary, AnalyzedFile, ImportDependency, ImportResolution, RepositoryGraph, Symbol,
        SymbolKind, SymbolReference, SymbolResolution,
    };

    use super::*;

    fn graph() -> RepositoryGraph {
        let authentication = Symbol {
            id: "sym-auth".to_owned(),
            name: "authenticate_user".to_owned(),
            kind: SymbolKind::Function,
            file_path: PathBuf::from("src/auth.rs"),
            start_byte: 0,
            end_byte: 50,
            start_line: 1,
            end_line: 4,
            parent_symbol_id: None,
        };
        let verify = Symbol {
            id: "sym-verify".to_owned(),
            name: "verify_token".to_owned(),
            kind: SymbolKind::Function,
            file_path: PathBuf::from("src/token.rs"),
            start_byte: 0,
            end_byte: 30,
            start_line: 1,
            end_line: 3,
            parent_symbol_id: None,
        };
        let unrelated = Symbol {
            id: "sym-unrelated".to_owned(),
            name: "audit_log".to_owned(),
            kind: SymbolKind::Function,
            file_path: PathBuf::from("src/audit.rs"),
            start_byte: 0,
            end_byte: 20,
            start_line: 1,
            end_line: 2,
            parent_symbol_id: None,
        };

        RepositoryGraph {
            schema_version: 1,
            repository_root: PathBuf::from("."),
            files: vec![
                AnalyzedFile {
                    path: PathBuf::from("src/auth.rs"),
                    language: crate::SourceLanguage::Rust,
                    size_bytes: 100,
                    line_count: 10,
                    has_syntax_errors: false,
                    syntax_error_count: 0,
                },
                AnalyzedFile {
                    path: PathBuf::from("src/token.rs"),
                    language: crate::SourceLanguage::Rust,
                    size_bytes: 80,
                    line_count: 8,
                    has_syntax_errors: false,
                    syntax_error_count: 0,
                },
                AnalyzedFile {
                    path: PathBuf::from("src/audit.rs"),
                    language: crate::SourceLanguage::Rust,
                    size_bytes: 40,
                    line_count: 4,
                    has_syntax_errors: false,
                    syntax_error_count: 0,
                },
            ],
            symbols: vec![authentication, verify, unrelated],
            imports: vec![ImportDependency {
                source_file: PathBuf::from("src/auth.rs"),
                target: "crate::token".to_owned(),
                kind: "use_declaration".to_owned(),
                line: 1,
                resolution: ImportResolution::Local,
                resolved_file: Some(PathBuf::from("src/token.rs")),
            }],
            references: vec![
                SymbolReference {
                    source_file: PathBuf::from("src/auth.rs"),
                    from_symbol_id: Some("sym-auth".to_owned()),
                    target_name: "verify_token".to_owned(),
                    line: 3,
                    resolved_symbol_id: Some("sym-verify".to_owned()),
                    resolution: SymbolResolution::UniqueNameMatch,
                },
                SymbolReference {
                    source_file: PathBuf::from("src/auth.rs"),
                    from_symbol_id: Some("sym-auth".to_owned()),
                    target_name: "audit_log".to_owned(),
                    line: 4,
                    resolved_symbol_id: Some("sym-unrelated".to_owned()),
                    resolution: SymbolResolution::UniqueNameMatch,
                },
            ],
            summary: AnalysisSummary::default(),
        }
    }

    #[test]
    fn recommends_symbols_using_identifier_terms() {
        let candidates = recommend_feature_seeds(&graph(), "authentication", "user login", 5);

        assert_eq!(candidates[0].symbol_id, "sym-auth");
        assert!(candidates[0].score > 0);
    }

    #[test]
    fn boundary_includes_transitive_import_and_call_dependencies() {
        let boundary = analyze_feature_boundary(
            &graph(),
            "authentication",
            "Authenticate a user",
            &["sym-auth".to_owned()],
        )
        .unwrap();

        assert_eq!(boundary.included_files.len(), 2);
        assert!(
            boundary
                .included_files
                .contains(&PathBuf::from("src/token.rs"))
        );
        assert_eq!(boundary.included_symbol_ids.len(), 2);
        assert!(
            !boundary
                .included_files
                .contains(&PathBuf::from("src/audit.rs"))
        );
        assert_eq!(boundary.unfollowed_symbol_references.len(), 1);
        assert_eq!(
            boundary.unfollowed_symbol_references[0].target_name,
            "audit_log"
        );
    }

    #[test]
    fn rejects_unknown_seed_ids() {
        let result = analyze_feature_boundary(
            &graph(),
            "authentication",
            "Authenticate a user",
            &["missing".to_owned()],
        );

        assert!(matches!(
            result,
            Err(BoundaryError::UnknownSeedSymbol(id)) if id == "missing"
        ));
    }

    #[test]
    fn rejects_empty_seeds() {
        let result = analyze_feature_boundary(&graph(), "authentication", "Authenticate", &[]);
        assert!(matches!(result, Err(BoundaryError::NoSeedSymbols)));
    }
}
