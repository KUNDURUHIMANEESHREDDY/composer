use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A normalized description of the application the agent is expected to compose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSpec {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub capabilities: Vec<CapabilityRequirement>,
    #[serde(default)]
    pub constraints: ProjectConstraints,
}

impl ProjectSpec {
    /// Validate user-controlled requirements before they enter an autonomous workflow.
    pub fn validate(&self) -> Result<(), SpecValidationError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(SpecValidationError::EmptyName);
        }
        if name.chars().count() > 80 {
            return Err(SpecValidationError::NameTooLong);
        }

        let description = self.description.trim();
        if description.is_empty() {
            return Err(SpecValidationError::EmptyDescription);
        }
        if description.chars().count() > 4_000 {
            return Err(SpecValidationError::DescriptionTooLong);
        }

        if self.capabilities.is_empty() {
            return Err(SpecValidationError::NoCapabilities);
        }

        let mut seen = HashSet::with_capacity(self.capabilities.len());
        for capability in &self.capabilities {
            if !is_valid_capability_id(&capability.id) {
                return Err(SpecValidationError::InvalidCapabilityId {
                    id: capability.id.clone(),
                });
            }
            if !seen.insert(capability.id.as_str()) {
                return Err(SpecValidationError::DuplicateCapabilityId {
                    id: capability.id.clone(),
                });
            }
            if capability.description.trim().is_empty() {
                return Err(SpecValidationError::EmptyCapabilityDescription {
                    id: capability.id.clone(),
                });
            }
        }

        for language in &self.constraints.preferred_languages {
            if language.trim().is_empty() {
                return Err(SpecValidationError::EmptyPreferredLanguage);
            }
        }

        Ok(())
    }
}

/// A capability requested by the user, such as authentication or file uploads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequirement {
    /// Stable machine-readable identifier, using lowercase kebab-case.
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub priority: CapabilityPriority,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPriority {
    Critical,
    High,
    #[default]
    Medium,
    Low,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectConstraints {
    /// Preferred implementation languages, not hard restrictions.
    pub preferred_languages: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SpecValidationError {
    #[error("project name must not be empty")]
    EmptyName,
    #[error("project name must be at most 80 characters")]
    NameTooLong,
    #[error("project description must not be empty")]
    EmptyDescription,
    #[error("project description must be at most 4000 characters")]
    DescriptionTooLong,
    #[error("at least one capability is required")]
    NoCapabilities,
    #[error("capability id '{id}' must use lowercase kebab-case")]
    InvalidCapabilityId { id: String },
    #[error("capability id '{id}' is duplicated")]
    DuplicateCapabilityId { id: String },
    #[error("capability '{id}' must have a description")]
    EmptyCapabilityDescription { id: String },
    #[error("preferred language entries must not be empty")]
    EmptyPreferredLanguage,
}

fn is_valid_capability_id(value: &str) -> bool {
    if value.is_empty() || value.starts_with('-') || value.ends_with('-') {
        return false;
    }

    let mut previous_was_hyphen = false;
    for character in value.chars() {
        if character == '-' {
            if previous_was_hyphen {
                return false;
            }
            previous_was_hyphen = true;
        } else if character.is_ascii_lowercase() || character.is_ascii_digit() {
            previous_was_hyphen = false;
        } else {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_spec() -> ProjectSpec {
        ProjectSpec {
            name: "Task Platform".to_owned(),
            description: "A task platform assembled from reusable capabilities.".to_owned(),
            capabilities: vec![CapabilityRequirement {
                id: "authentication".to_owned(),
                description: "User sign-in and session management.".to_owned(),
                priority: CapabilityPriority::Critical,
            }],
            constraints: ProjectConstraints {
                preferred_languages: vec!["rust".to_owned(), "typescript".to_owned()],
            },
        }
    }

    #[test]
    fn accepts_a_valid_specification() {
        assert_eq!(valid_spec().validate(), Ok(()));
    }

    #[test]
    fn rejects_a_specification_without_capabilities() {
        let mut spec = valid_spec();
        spec.capabilities.clear();

        assert_eq!(spec.validate(), Err(SpecValidationError::NoCapabilities));
    }

    #[test]
    fn rejects_duplicate_capability_ids() {
        let mut spec = valid_spec();
        spec.capabilities.push(spec.capabilities[0].clone());

        assert_eq!(
            spec.validate(),
            Err(SpecValidationError::DuplicateCapabilityId {
                id: "authentication".to_owned()
            })
        );
    }

    #[test]
    fn rejects_non_kebab_case_capability_ids() {
        let mut spec = valid_spec();
        spec.capabilities[0].id = "AuthenticationService".to_owned();

        assert_eq!(
            spec.validate(),
            Err(SpecValidationError::InvalidCapabilityId {
                id: "AuthenticationService".to_owned()
            })
        );
    }

    #[test]
    fn rejects_empty_capability_descriptions() {
        let mut spec = valid_spec();
        spec.capabilities[0].description = "  ".to_owned();

        assert_eq!(
            spec.validate(),
            Err(SpecValidationError::EmptyCapabilityDescription {
                id: "authentication".to_owned()
            })
        );
    }

    #[test]
    fn rejects_empty_preferred_language_entries() {
        let mut spec = valid_spec();
        spec.constraints.preferred_languages.push(" ".to_owned());

        assert_eq!(
            spec.validate(),
            Err(SpecValidationError::EmptyPreferredLanguage)
        );
    }
}
