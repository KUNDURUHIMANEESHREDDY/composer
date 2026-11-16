use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::SourceLanguage;

pub const GRAPH_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryGraph {
    pub schema_version: u32,
    pub repository_root: PathBuf,
    pub files: Vec<AnalyzedFile>,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<ImportDependency>,
    pub references: Vec<SymbolReference>,
    pub summary: AnalysisSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzedFile {
    pub path: PathBuf,
    pub language: SourceLanguage,
    pub size_bytes: u64,
    pub line_count: usize,
    pub has_syntax_errors: bool,
    pub syntax_error_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: String,
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
    /// One-based source line.
    pub start_line: usize,
    /// One-based inclusive source line.
    pub end_line: usize,
    pub parent_symbol_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    TypeAlias,
    Module,
    Constant,
    Variable,
    Macro,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportDependency {
    pub source_file: PathBuf,
    pub target: String,
    pub kind: String,
    pub line: usize,
    pub resolution: ImportResolution,
    pub resolved_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportResolution {
    Local,
    UnresolvedOrExternal,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolReference {
    pub source_file: PathBuf,
    pub from_symbol_id: Option<String>,
    pub target_name: String,
    pub line: usize,
    pub resolved_symbol_id: Option<String>,
    pub resolution: SymbolResolution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolResolution {
    UniqueNameMatch,
    AmbiguousNameMatch,
    Unresolved,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalysisSummary {
    pub files_analyzed: usize,
    pub files_skipped: usize,
    pub languages: Vec<SourceLanguage>,
    pub symbols_found: usize,
    pub imports_found: usize,
    pub local_imports_resolved: usize,
    pub unresolved_imports: usize,
    pub ambiguous_imports: usize,
    pub symbol_references_found: usize,
    pub symbol_references_resolved: usize,
    pub syntax_files_with_errors: usize,
    pub inventory_truncated: bool,
}
