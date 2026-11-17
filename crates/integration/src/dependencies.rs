use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_MANIFEST_FILES: usize = 2_000;
const MAX_MANIFEST_BYTES: u64 = 2_000_000;
const MAX_DIRECTORY_DEPTH: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyEcosystem {
    Cargo,
    Npm,
    Python,
    Go,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    Runtime,
    Development,
    Build,
    Optional,
    Peer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyRequirement {
    pub ecosystem: DependencyEcosystem,
    pub name: String,
    pub requirement: String,
    pub kind: DependencyKind,
    pub source_manifest: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseDeclaration {
    pub ecosystem: DependencyEcosystem,
    pub package_name: Option<String>,
    pub expression: String,
    pub source_manifest: PathBuf,
    /// Checks expression grammar only. It does not validate SPDX identifiers
    /// against the official SPDX License List and is not a legal conclusion.
    pub syntax_valid: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyInventory {
    pub manifests_inspected: Vec<PathBuf>,
    pub dependencies: Vec<DependencyRequirement>,
    pub license_declarations: Vec<LicenseDeclaration>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    AlreadyPresent,
    NotDeclaredInTarget,
    DifferentConstraint,
}

#[derive(Debug, Error)]
pub enum DependencyError {
    #[error("manifest inventory filesystem operation failed: {0}")]
    Filesystem(#[from] std::io::Error),
    #[error("manifest is too large: {0}")]
    ManifestTooLarge(PathBuf),
}

pub(crate) fn discover_manifest_paths(root: &Path) -> Result<Vec<PathBuf>, DependencyError> {
    let mut output = BTreeSet::new();
    let mut pending = VecDeque::from([(PathBuf::new(), 0usize)]);

    while let Some((relative_dir, depth)) = pending.pop_front() {
        if depth > MAX_DIRECTORY_DEPTH {
            continue;
        }

        let absolute_dir = root.join(&relative_dir);
        let mut entries = fs::read_dir(&absolute_dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }

            let name = entry.file_name().to_string_lossy().into_owned();
            if file_type.is_dir() {
                if should_skip_directory(&name) {
                    continue;
                }
                pending.push_back((relative_dir.join(name), depth + 1));
                continue;
            }

            if file_type.is_file() && is_package_manifest(&name) {
                output.insert(relative_dir.join(name));
                if output.len() >= MAX_MANIFEST_FILES {
                    return Ok(output.into_iter().collect());
                }
            }
        }
    }

    Ok(output.into_iter().collect())
}

pub(crate) fn inventory_manifests(
    root: &Path,
    relative_paths: &[PathBuf],
) -> Result<DependencyInventory, DependencyError> {
    let mut inventory = DependencyInventory::default();

    for relative in relative_paths {
        if !is_safe_relative_path(relative) {
            inventory.warnings.push(format!(
                "Skipped unsafe manifest path '{}'.",
                relative.display()
            ));
            continue;
        }

        let path = root.join(relative);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file() {
            inventory.warnings.push(format!(
                "Skipped non-regular package manifest '{}'.",
                relative.display()
            ));
            continue;
        }
        if metadata.len() > MAX_MANIFEST_BYTES {
            return Err(DependencyError::ManifestTooLarge(relative.clone()));
        }

        let content = fs::read_to_string(&path)?;
        inventory.manifests_inspected.push(relative.clone());
        parse_manifest(relative, &content, &mut inventory);
    }

    inventory.dependencies.sort_by(|left, right| {
        left.ecosystem
            .cmp(&right.ecosystem)
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
            .then_with(|| left.source_manifest.cmp(&right.source_manifest))
    });
    inventory.dependencies.dedup_by(|left, right| {
        left.ecosystem == right.ecosystem
            && left.name.eq_ignore_ascii_case(&right.name)
            && left.requirement == right.requirement
            && left.kind == right.kind
            && left.source_manifest == right.source_manifest
    });
    inventory.manifests_inspected.sort();
    Ok(inventory)
}

fn parse_manifest(relative: &Path, content: &str, inventory: &mut DependencyInventory) {
    match relative.file_name().and_then(|name| name.to_str()) {
        Some("Cargo.toml") => parse_cargo_manifest(relative, content, inventory),
        Some("package.json") => parse_npm_manifest(relative, content, inventory),
        Some("pyproject.toml") => parse_pyproject_manifest(relative, content, inventory),
        Some("requirements.txt") => parse_requirements(relative, content, inventory),
        Some("go.mod") => parse_go_mod(relative, content, inventory),
        _ => {}
    }
}

fn parse_cargo_manifest(relative: &Path, content: &str, inventory: &mut DependencyInventory) {
    let value: toml::Value = match toml::from_str(content) {
        Ok(value) => value,
        Err(error) => {
            inventory.warnings.push(format!(
                "Could not parse Cargo manifest '{}': {error}",
                relative.display()
            ));
            return;
        }
    };

    let package = value.get("package");
    let package_name = package
        .and_then(|table| table.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned);

    if let Some(expression) = package
        .and_then(|table| table.get("license"))
        .and_then(toml::Value::as_str)
    {
        inventory.license_declarations.push(license_declaration(
            DependencyEcosystem::Cargo,
            package_name.clone(),
            expression,
            relative,
        ));
    }

    add_toml_dependency_table(
        value.get("dependencies"),
        DependencyKind::Runtime,
        relative,
        inventory,
    );
    add_toml_dependency_table(
        value.get("dev-dependencies"),
        DependencyKind::Development,
        relative,
        inventory,
    );
    add_toml_dependency_table(
        value.get("build-dependencies"),
        DependencyKind::Build,
        relative,
        inventory,
    );
    add_toml_dependency_table(
        value
            .get("workspace")
            .and_then(|table| table.get("dependencies")),
        DependencyKind::Runtime,
        relative,
        inventory,
    );

    if let Some(targets) = value.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            add_toml_dependency_table(
                target.get("dependencies"),
                DependencyKind::Runtime,
                relative,
                inventory,
            );
            add_toml_dependency_table(
                target.get("dev-dependencies"),
                DependencyKind::Development,
                relative,
                inventory,
            );
        }
    }
}

fn add_toml_dependency_table(
    table: Option<&toml::Value>,
    kind: DependencyKind,
    relative: &Path,
    inventory: &mut DependencyInventory,
) {
    let Some(table) = table.and_then(toml::Value::as_table) else {
        return;
    };

    for (name, value) in table {
        let requirement = if let Some(version) = value.as_str() {
            version.to_owned()
        } else if let Some(options) = value.as_table() {
            let mut parts = Vec::new();
            for key in ["version", "path", "git", "branch", "tag", "rev"] {
                if let Some(value) = options.get(key).and_then(toml::Value::as_str) {
                    parts.push(format!("{key}={value}"));
                }
            }
            parts.join(",")
        } else {
            String::new()
        };

        inventory.dependencies.push(DependencyRequirement {
            ecosystem: DependencyEcosystem::Cargo,
            name: name.clone(),
            requirement,
            kind,
            source_manifest: relative.to_path_buf(),
        });
    }
}

fn parse_npm_manifest(relative: &Path, content: &str, inventory: &mut DependencyInventory) {
    let value: serde_json::Value = match serde_json::from_str(content) {
        Ok(value) => value,
        Err(error) => {
            inventory.warnings.push(format!(
                "Could not parse package manifest '{}': {error}",
                relative.display()
            ));
            return;
        }
    };

    let package_name = value
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    if let Some(expression) = value.get("license").and_then(|value| value.as_str()) {
        inventory.license_declarations.push(license_declaration(
            DependencyEcosystem::Npm,
            package_name,
            expression,
            relative,
        ));
    }

    add_json_dependencies(
        value.get("dependencies"),
        DependencyKind::Runtime,
        relative,
        inventory,
    );
    add_json_dependencies(
        value.get("devDependencies"),
        DependencyKind::Development,
        relative,
        inventory,
    );
    add_json_dependencies(
        value.get("peerDependencies"),
        DependencyKind::Peer,
        relative,
        inventory,
    );
    add_json_dependencies(
        value.get("optionalDependencies"),
        DependencyKind::Optional,
        relative,
        inventory,
    );
}

fn add_json_dependencies(
    value: Option<&serde_json::Value>,
    kind: DependencyKind,
    relative: &Path,
    inventory: &mut DependencyInventory,
) {
    let Some(dependencies) = value.and_then(serde_json::Value::as_object) else {
        return;
    };

    for (name, value) in dependencies {
        inventory.dependencies.push(DependencyRequirement {
            ecosystem: DependencyEcosystem::Npm,
            name: name.clone(),
            requirement: value.as_str().unwrap_or_default().to_owned(),
            kind,
            source_manifest: relative.to_path_buf(),
        });
    }
}

fn parse_pyproject_manifest(relative: &Path, content: &str, inventory: &mut DependencyInventory) {
    let value: toml::Value = match toml::from_str(content) {
        Ok(value) => value,
        Err(error) => {
            inventory.warnings.push(format!(
                "Could not parse Python project manifest '{}': {error}",
                relative.display()
            ));
            return;
        }
    };

    let project = value.get("project");
    let package_name = project
        .and_then(|table| table.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned);

    if let Some(expression) = project
        .and_then(|table| table.get("license"))
        .and_then(toml::Value::as_str)
    {
        inventory.license_declarations.push(license_declaration(
            DependencyEcosystem::Python,
            package_name,
            expression,
            relative,
        ));
    }

    if let Some(dependencies) = project
        .and_then(|table| table.get("dependencies"))
        .and_then(toml::Value::as_array)
    {
        for dependency in dependencies.iter().filter_map(toml::Value::as_str) {
            if let Some((name, requirement)) = parse_pep508_name(dependency) {
                inventory.dependencies.push(DependencyRequirement {
                    ecosystem: DependencyEcosystem::Python,
                    name,
                    requirement,
                    kind: DependencyKind::Runtime,
                    source_manifest: relative.to_path_buf(),
                });
            }
        }
    }

    if let Some(optional) = project
        .and_then(|table| table.get("optional-dependencies"))
        .and_then(toml::Value::as_table)
    {
        for group in optional.values().filter_map(toml::Value::as_array) {
            for dependency in group.iter().filter_map(toml::Value::as_str) {
                if let Some((name, requirement)) = parse_pep508_name(dependency) {
                    inventory.dependencies.push(DependencyRequirement {
                        ecosystem: DependencyEcosystem::Python,
                        name,
                        requirement,
                        kind: DependencyKind::Optional,
                        source_manifest: relative.to_path_buf(),
                    });
                }
            }
        }
    }
}

fn parse_pep508_name(value: &str) -> Option<(String, String)> {
    let value = value.trim();
    let name_end = value
        .find(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
        })
        .unwrap_or(value.len());
    if name_end == 0 {
        return None;
    }
    let name = value[..name_end].to_owned();
    Some((name, value[name_end..].trim().to_owned()))
}

fn parse_requirements(relative: &Path, content: &str, inventory: &mut DependencyInventory) {
    for (line_number, raw_line) in content.lines().enumerate() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('-') || line.contains("://") || line.contains(" @ ") {
            inventory.warnings.push(format!(
                "Requirement at {}:{} uses an option or direct URL and needs manual review: {}",
                relative.display(),
                line_number + 1,
                line
            ));
            continue;
        }
        if let Some((name, requirement)) = parse_pep508_name(line) {
            inventory.dependencies.push(DependencyRequirement {
                ecosystem: DependencyEcosystem::Python,
                name,
                requirement,
                kind: DependencyKind::Runtime,
                source_manifest: relative.to_path_buf(),
            });
        }
    }
}

fn parse_go_mod(relative: &Path, content: &str, inventory: &mut DependencyInventory) {
    let mut in_require_block = false;
    for (line_number, raw_line) in content.lines().enumerate() {
        let line = raw_line.split("//").next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line == "require (" {
            in_require_block = true;
            continue;
        }
        if in_require_block && line == ")" {
            in_require_block = false;
            continue;
        }

        let requirement_line = if in_require_block {
            Some(line)
        } else {
            line.strip_prefix("require ")
        };
        let Some(requirement_line) = requirement_line else {
            continue;
        };

        let mut parts = requirement_line.split_whitespace();
        let Some(name) = parts.next() else {
            continue;
        };
        let Some(version) = parts.next() else {
            inventory.warnings.push(format!(
                "Malformed Go dependency at {}:{}.",
                relative.display(),
                line_number + 1
            ));
            continue;
        };

        inventory.dependencies.push(DependencyRequirement {
            ecosystem: DependencyEcosystem::Go,
            name: name.to_owned(),
            requirement: version.to_owned(),
            kind: DependencyKind::Runtime,
            source_manifest: relative.to_path_buf(),
        });
    }
}

fn license_declaration(
    ecosystem: DependencyEcosystem,
    package_name: Option<String>,
    expression: &str,
    relative: &Path,
) -> LicenseDeclaration {
    LicenseDeclaration {
        ecosystem,
        package_name,
        expression: expression.trim().to_owned(),
        source_manifest: relative.to_path_buf(),
        syntax_valid: is_spdx_expression_syntactically_valid(expression),
    }
}

fn is_spdx_expression_syntactically_valid(expression: &str) -> bool {
    let tokens = tokenize_spdx(expression);
    if tokens.is_empty() {
        return false;
    }
    let mut parser = SpdxParser { tokens, index: 0 };
    parser.parse_or() && parser.index == parser.tokens.len()
}

fn tokenize_spdx(expression: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for character in expression.chars() {
        match character {
            '(' | ')' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(character.to_string());
            }
            character if character.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            character
                if character.is_ascii_alphanumeric()
                    || matches!(character, '-' | '.' | '+' | ':') =>
            {
                current.push(character);
            }
            _ => return Vec::new(),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

struct SpdxParser {
    tokens: Vec<String>,
    index: usize,
}

impl SpdxParser {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.index).map(String::as_str)
    }

    fn consume(&mut self, expected: &str) -> bool {
        if self.peek() == Some(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn parse_or(&mut self) -> bool {
        if !self.parse_and() {
            return false;
        }
        while self.consume("OR") {
            if !self.parse_and() {
                return false;
            }
        }
        true
    }

    fn parse_and(&mut self) -> bool {
        if !self.parse_with() {
            return false;
        }
        while self.consume("AND") {
            if !self.parse_with() {
                return false;
            }
        }
        true
    }

    fn parse_with(&mut self) -> bool {
        if !self.parse_primary() {
            return false;
        }
        if self.consume("WITH") {
            return self.consume_identifier();
        }
        true
    }

    fn parse_primary(&mut self) -> bool {
        if self.consume("(") {
            if !self.parse_or() || !self.consume(")") {
                return false;
            }
            true
        } else {
            self.consume_identifier()
        }
    }

    fn consume_identifier(&mut self) -> bool {
        let Some(value) = self.peek() else {
            return false;
        };
        if matches!(value, "AND" | "OR" | "WITH" | "(" | ")") || value.is_empty() {
            return false;
        }
        self.index += 1;
        true
    }
}

fn is_package_manifest(name: &str) -> bool {
    matches!(
        name,
        "Cargo.toml" | "package.json" | "pyproject.toml" | "requirements.txt" | "go.mod"
    )
}

fn should_skip_directory(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".next"
            | ".turbo"
            | ".venv"
            | "node_modules"
            | "target"
            | "vendor"
            | "dist"
            | "build"
            | ".composer"
            | "__pycache__"
    ) || name.starts_with('.')
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spdx_boolean_expressions_without_claiming_identifier_validation() {
        assert!(is_spdx_expression_syntactically_valid("MIT OR Apache-2.0"));
        assert!(is_spdx_expression_syntactically_valid(
            "MIT AND (BSD-2-Clause OR Apache-2.0)"
        ));
        assert!(is_spdx_expression_syntactically_valid(
            "GPL-3.0-only WITH Classpath-exception-2.0"
        ));
        assert!(!is_spdx_expression_syntactically_valid("MIT OR"));
        assert!(!is_spdx_expression_syntactically_valid("MIT && Apache-2.0"));
        assert!(!is_spdx_expression_syntactically_valid("(MIT"));
    }

    #[test]
    fn parses_pep508_names_and_constraints() {
        let (name, constraint) =
            parse_pep508_name("requests>=2.31; python_version >= '3.10'").unwrap();
        assert_eq!(name, "requests");
        assert!(constraint.starts_with(">="));
    }

    #[test]
    fn recognizes_supported_manifest_files() {
        assert!(is_package_manifest("Cargo.toml"));
        assert!(is_package_manifest("package.json"));
        assert!(is_package_manifest("pyproject.toml"));
        assert!(is_package_manifest("requirements.txt"));
        assert!(is_package_manifest("go.mod"));
        assert!(!is_package_manifest("Cargo.lock"));
    }
}

#[test]
fn inventories_dependencies_across_supported_package_manifests() {
    let root = std::env::temp_dir().join(format!("composer-dependencies-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"cargo-fixture\"\nversion = \"0.1.0\"\nlicense = \"MIT OR Apache-2.0\"\n[dependencies]\nserde = \"1\"\n[dev-dependencies]\ntempfile = \"3\"\n",
        )
        .unwrap();
    fs::write(
            root.join("package.json"),
            "{\"name\":\"web-fixture\",\"license\":\"MIT\",\"dependencies\":{\"express\":\"^4.0\"},\"devDependencies\":{\"vitest\":\"^2.0\"}}",
        )
        .unwrap();
    fs::write(
            root.join("pyproject.toml"),
            "[project]\nname = \"python-fixture\"\nlicense = \"BSD-3-Clause\"\ndependencies = [\"fastapi>=0.110\", \"pydantic[extra]>=2\"]\n[project.optional-dependencies]\ntest = [\"pytest>=8\"]\n",
        )
        .unwrap();
    fs::write(root.join("requirements.txt"), "requests==2.32\n").unwrap();
    fs::write(
            root.join("go.mod"),
            "module example.com/fixture\ngo 1.22\nrequire (\n github.com/example/lib v1.2.3\n github.com/example/testkit v0.4.0 // indirect\n)\n",
        )
        .unwrap();

    let paths = vec![
        PathBuf::from("Cargo.toml"),
        PathBuf::from("package.json"),
        PathBuf::from("pyproject.toml"),
        PathBuf::from("requirements.txt"),
        PathBuf::from("go.mod"),
    ];
    let inventory = inventory_manifests(&root, &paths).unwrap();

    assert!(
        inventory
            .dependencies
            .iter()
            .any(|item| { item.ecosystem == DependencyEcosystem::Cargo && item.name == "serde" })
    );
    assert!(
        inventory
            .dependencies
            .iter()
            .any(|item| { item.ecosystem == DependencyEcosystem::Npm && item.name == "express" })
    );
    assert!(
        inventory.dependencies.iter().any(|item| {
            item.ecosystem == DependencyEcosystem::Python && item.name == "fastapi"
        })
    );
    assert!(
        inventory
            .dependencies
            .iter()
            .any(|item| { item.ecosystem == DependencyEcosystem::Python && item.name == "pytest" })
    );
    assert!(inventory.dependencies.iter().any(|item| {
        item.ecosystem == DependencyEcosystem::Go && item.name == "github.com/example/lib"
    }));
    assert!(
        inventory
            .license_declarations
            .iter()
            .any(|item| { item.expression == "MIT OR Apache-2.0" && item.syntax_valid })
    );
    assert!(
        inventory
            .license_declarations
            .iter()
            .any(|item| { item.expression == "BSD-3-Clause" && item.syntax_valid })
    );

    let _ = fs::remove_dir_all(root);
}
