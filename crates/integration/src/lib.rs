//! Independent Feature Capsule verification, dependency/license inventory,
//! target compatibility analysis, and safe isolated integration staging.

mod apply;
mod batch;
mod compatibility;
mod dependencies;
mod multifile;
mod verified;
mod verify;
mod wire;

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
pub use verified::{
    VerifiedRustError, VerifiedRustOptions, VerifiedRustResult, compose_verified_rust,
};
pub use verify::{CapsuleVerification, VerificationError, VerifiedCapsule, verify_capsule};
pub use wire::{RustWiringError, RustWiringOptions, RustWiringResult, wire_rust_module};
