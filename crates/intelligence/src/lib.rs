//! Source analysis, dependency graph construction, feature boundaries, and capsules.
//!
//! Tree-sitter provides syntax structure. Cross-file import and symbol resolution
//! are evidence-backed heuristics, not compiler-grade semantic guarantees.

mod analyzer;
mod boundary;
mod capsule;
mod language;
mod model;
mod parser;

pub use analyzer::{AnalysisError, analyze_repository};
pub use boundary::{
    BoundaryError, FeatureBoundary, FeatureCandidate, analyze_feature_boundary,
    recommend_feature_seeds,
};
pub use capsule::{
    CapsuleError, CapsuleFile, CapsuleManifest, CapsuleOptions, CapsuleResult,
    create_feature_capsule,
};
pub use language::SourceLanguage;
pub use model::{
    AnalysisSummary, AnalyzedFile, ImportDependency, ImportResolution, RepositoryGraph, Symbol,
    SymbolKind, SymbolReference, SymbolResolution,
};
