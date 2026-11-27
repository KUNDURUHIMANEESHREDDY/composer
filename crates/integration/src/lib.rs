//! Independent Feature Capsule verification, dependency/license inventory,
//! target compatibility analysis, and safe isolated integration staging.

#![recursion_limit = "256"]
mod api_discovery;
mod api_signature;
mod apply;
mod batch;
mod compatibility;
mod dependencies;
mod external_discovery;
mod multifile;
mod reviewed;
mod reviewed_upstream_adapter;
mod upstream_crate;
mod upstream_git;
mod upstream_recovery;
mod verified;
mod verify;
mod wire;

pub use api_discovery::{
    RustApiDiscoveryError, RustApiDiscoveryReport, RustApiSuggestion, RustTargetRequirement,
    discover_rust_api_contracts,
};
pub use apply::{IntegrationError, IntegrationOptions, IntegrationResult, apply_integration};
pub use batch::{
    BatchRustError, BatchRustOptions, BatchRustResult, BatchStep, compose_verified_rust_batch,
};
pub use compatibility::{
    AdapterContract, CompatibilityError, CompatibilityFinding, CompatibilityLevel,
    CompatibilityPlan, IntegrationStrategy, SymbolCollision, TargetProfile, TestEvidence,
    plan_integration,
};
pub use dependencies::{
    DependencyEcosystem, DependencyInventory, DependencyKind, DependencyRequirement,
    LicenseDeclaration, RequirementStatus,
};
pub use external_discovery::{
    ExternalDiscoveryOptions, ExternalDiscoveryResult, discover_external_rust,
};
pub use upstream_crate::{UpstreamCrateOptions, UpstreamCrateResult, verify_upstream_crate};
pub use upstream_git::{UpstreamGitOptions, UpstreamGitResult, verify_upstream_git};
pub use upstream_recovery::{
    UpstreamRecoveryAttempt, UpstreamRecoveryCandidate, UpstreamRecoveryOptions,
    UpstreamRecoveryResult, recover_upstream_crate,
};
pub use verified::{
    VerifiedRustError, VerifiedRustOptions, VerifiedRustResult, compose_verified_rust,
};
pub use verify::{CapsuleVerification, VerificationError, VerifiedCapsule, verify_capsule};
pub use wire::{RustWiringError, RustWiringOptions, RustWiringResult, wire_rust_module};
