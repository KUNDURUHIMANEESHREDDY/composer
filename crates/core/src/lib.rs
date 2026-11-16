//! Shared domain types and deterministic validation for Software Composer.

mod specification;

pub use specification::{
    CapabilityPriority, CapabilityRequirement, ProjectConstraints, ProjectSpec, SpecValidationError,
};
