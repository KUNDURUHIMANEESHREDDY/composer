//! Independent Feature Capsule verification, dependency/license inventory,
//! target compatibility analysis, and safe isolated integration staging.

mod apply;
mod compatibility;
mod dependencies;
mod verify;

pub use apply::{IntegrationError, IntegrationOptions, IntegrationResult, apply_integration};
pub use compatibility::{
    AdapterContract, CompatibilityError, CompatibilityFinding, CompatibilityLevel,
    CompatibilityPlan, IntegrationStrategy, SymbolCollision, TargetProfile, TestEvidence,
    plan_integration,
};
pub use dependencies::{
    DependencyEcosystem, DependencyInventory, DependencyKind, DependencyRequirement,
    LicenseDeclaration, RequirementStatus,
};
pub use verify::{CapsuleVerification, VerificationError, VerifiedCapsule, verify_capsule};
