//! Safe acquisition and basic inventory of external Git repositories.
//!
//! This crate does not execute repository code. It only clones source and reads
//! Git metadata and recognized manifest filenames.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

const MAX_INVENTORY_FILES: usize = 20_000;
const MAX_INVENTORY_DEPTH: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepository {
    pub owner: String,
    pub name: String,
    pub clone_url: String,
}

impl GitHubRepository {
    /// Parse a repository URL, accepting only canonical HTTPS github.com URLs.
    pub fn parse(input: &str) -> Result<Self, RepositoryError> {
        let parsed = Url::parse(input).map_err(|_| RepositoryError::InvalidGitHubUrl)?;

        if parsed.scheme() != "https"
            || parsed.host_str() != Some("github.com")
            || parsed.port().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(RepositoryError::InvalidGitHubUrl);
        }

        let segments: Vec<&str> = parsed
            .path_segments()
            .ok_or(RepositoryError::InvalidGitHubUrl)?
            .filter(|segment| !segment.is_empty())
            .collect();

        if segments.len() != 2 {
            return Err(RepositoryError::InvalidGitHubUrl);
        }

        let owner = segments[0];
        let raw_name = segments[1];
        let name = raw_name.strip_suffix(".git").unwrap_or(raw_name);

        if !is_valid_owner(owner) || !is_valid_repository_name(name) {
            return Err(RepositoryError::InvalidGitHubUrl);
        }

        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
            clone_url: format!("https://github.com/{owner}/{name}.git"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInspection {
    pub repository_url: String,
    pub owner: String,
    pub name: String,
    pub commit_sha: String,
    /// Git tree object ID for the checked-out commit.
    pub tree_sha: String,
    pub checkout_path: PathBuf,
    pub manifest_files: Vec<String>,
    pub detected_languages: Vec<String>,
    pub inventory_truncated: bool,
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("expected an HTTPS GitHub repository URL such as https://github.com/owner/repository")]
    InvalidGitHubUrl,
    #[error("could not create repository workspace: {0}")]
    WorkspaceIo(#[from] std::io::Error),
    #[error("Git is unavailable or could not be started: {0}")]
    GitUnavailable(std::io::Error),
    #[error("Git clone failed: {0}")]
    CloneFailed(String),
    #[error("Git command '{command}' failed: {stderr}")]
    GitMetadataFailed {
        command: &'static str,
        stderr: String,
    },
}

/// Clone and inspect a public GitHub repository without executing project code.
///
/// The destination is always a newly generated child of the workspace root;
/// user input is never used as a filesystem path.
pub fn acquire_and_inspect(
    repository_url: &str,
    workspace_root: &Path,
) -> Result<RepositoryInspection, RepositoryError> {
    let repository = GitHubRepository::parse(repository_url)?;
    fs::create_dir_all(workspace_root)?;

    // Keep the caller's path spelling; Git on Windows may reject extended canonical paths.
    let checkout_path = workspace_root.join(format!(
        "{}-{}",
        repository.name.to_ascii_lowercase(),
        Uuid::new_v4()
    ));

    let clone_output = git_command()
        .args([
            "-c",
            "protocol.ext.allow=never",
            "clone",
            "--depth",
            "1",
            "--no-tags",
            "--no-recurse-submodules",
            "--",
        ])
        .arg(&repository.clone_url)
        .arg(&checkout_path)
        .output()
        .map_err(RepositoryError::GitUnavailable)?;

    if !clone_output.status.success() {
        let detail = output_detail(&clone_output);
        let _ = fs::remove_dir_all(&checkout_path);
        return Err(RepositoryError::CloneFailed(detail));
    }

    let commit_sha = git_output(
        &checkout_path,
        &["rev-parse", "--verify", "HEAD^{commit}"],
        "rev-parse",
    )?;
    let tree_sha = git_output(
        &checkout_path,
        &["rev-parse", "--verify", "HEAD^{tree}"],
        "rev-parse",
    )?;

    let (manifest_files, detected_languages, inventory_truncated) =
        inventory_manifests(&checkout_path)?;

    Ok(RepositoryInspection {
        repository_url: repository.clone_url,
        owner: repository.owner,
        name: repository.name,
        commit_sha,
        tree_sha,
        checkout_path,
        manifest_files,
        detected_languages,
        inventory_truncated,
    })
}

fn git_command() -> Command {
    let mut command = Command::new("git");
    // Highest-precedence native Git options block checkout-local fsmonitor
    // execution and unnecessary index caching in all host Git invocations.
    command.args([
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.untrackedCache=false",
    ]);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_ALLOW_PROTOCOL", "https")
        // Do not permit inherited Git config/environment to inject host
        // filters, templates or external helper execution at clone time.
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GIT_TEMPLATE_DIR");

    #[cfg(windows)]
    command.env("GIT_CONFIG_GLOBAL", "NUL");

    #[cfg(not(windows))]
    command.env("GIT_CONFIG_GLOBAL", "/dev/null");

    command
}

fn git_output(
    repository_path: &Path,
    arguments: &[&str],
    command_name: &'static str,
) -> Result<String, RepositoryError> {
    let output = git_command()
        .arg("-C")
        .arg(repository_path)
        .args(arguments)
        .output()
        .map_err(RepositoryError::GitUnavailable)?;

    if !output.status.success() {
        return Err(RepositoryError::GitMetadataFailed {
            command: command_name,
            stderr: output_detail(&output),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn output_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    } else {
        stderr
    }
}

fn is_valid_owner(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 39
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
}

fn is_valid_repository_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 100
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn inventory_manifests(
    repository_root: &Path,
) -> Result<(Vec<String>, Vec<String>, bool), RepositoryError> {
    let mut manifests = Vec::new();
    let mut visited_files = 0usize;
    let mut truncated = false;

    walk_manifests(
        repository_root,
        repository_root,
        0,
        &mut visited_files,
        &mut manifests,
        &mut truncated,
    )?;

    manifests.sort();

    let mut languages = Vec::new();
    for manifest in &manifests {
        let file_name = Path::new(manifest)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();

        let language = match file_name {
            "Cargo.toml" => Some("rust"),
            "package.json" => Some("javascript"),
            "tsconfig.json" => Some("typescript"),
            "pyproject.toml" | "requirements.txt" => Some("python"),
            "go.mod" => Some("go"),
            "pom.xml" | "build.gradle" | "build.gradle.kts" => Some("java"),
            "composer.json" => Some("php"),
            "Gemfile" => Some("ruby"),
            "mix.exs" => Some("elixir"),
            _ if file_name.ends_with(".csproj") => Some("csharp"),
            _ => None,
        };

        if let Some(language) = language
            && !languages.iter().any(|existing| existing == language)
        {
            languages.push(language.to_owned());
        }
    }

    languages.sort();
    Ok((manifests, languages, truncated))
}

fn walk_manifests(
    root: &Path,
    current: &Path,
    depth: usize,
    visited_files: &mut usize,
    manifests: &mut Vec<String>,
    truncated: &mut bool,
) -> Result<(), RepositoryError> {
    if depth > MAX_INVENTORY_DEPTH || *truncated {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if file_type.is_symlink() || name == ".git" || should_skip_directory(&name) {
            continue;
        }

        let path = entry.path();
        if file_type.is_dir() {
            walk_manifests(root, &path, depth + 1, visited_files, manifests, truncated)?;
            if *truncated {
                break;
            }
        } else if file_type.is_file() {
            *visited_files += 1;
            if *visited_files > MAX_INVENTORY_FILES {
                *truncated = true;
                break;
            }

            if is_manifest_name(&name) {
                let relative = path
                    .strip_prefix(root)
                    .expect("walked path is within repository root")
                    .to_string_lossy()
                    .replace('\\', "/");
                manifests.push(relative);
            }
        }
    }

    Ok(())
}

fn should_skip_directory(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | "vendor" | "target" | "dist" | ".next" | ".venv" | "__pycache__"
    )
}

fn is_manifest_name(name: &str) -> bool {
    matches!(
        name,
        "Cargo.toml"
            | "package.json"
            | "tsconfig.json"
            | "pyproject.toml"
            | "requirements.txt"
            | "go.mod"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "composer.json"
            | "Gemfile"
            | "mix.exs"
    ) || name.ends_with(".csproj")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_git_metadata_overrides_checkout_fsmonitor_cross_platform() {
        let base = std::env::temp_dir().join(format!("composer-host-git-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let init = Command::new("git")
            .arg("-C")
            .arg(&base)
            .args(["init", "-q"])
            .output()
            .unwrap();
        assert!(init.status.success());
        let config = Command::new("git")
            .arg("-C")
            .arg(&base)
            .args(["config", "core.fsmonitor", "true"])
            .output()
            .unwrap();
        assert!(config.status.success());
        let checked = git_command()
            .arg("-C")
            .arg(&base)
            .args(["config", "--get", "core.fsmonitor"])
            .output()
            .unwrap();
        assert!(checked.status.success());
        assert_eq!(String::from_utf8_lossy(&checked.stdout).trim(), "false");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn parses_canonical_github_https_url() {
        let repository = GitHubRepository::parse("https://github.com/example/project.git")
            .expect("URL should be accepted");

        assert_eq!(repository.owner, "example");
        assert_eq!(repository.name, "project");
        assert_eq!(
            repository.clone_url,
            "https://github.com/example/project.git"
        );
    }

    #[test]
    fn normalizes_trailing_slash() {
        let repository = GitHubRepository::parse("https://github.com/example/project/")
            .expect("URL should be accepted");

        assert_eq!(
            repository.clone_url,
            "https://github.com/example/project.git"
        );
    }

    #[test]
    fn rejects_non_github_hosts() {
        assert!(matches!(
            GitHubRepository::parse("https://gitlab.com/example/project"),
            Err(RepositoryError::InvalidGitHubUrl)
        ));
    }

    #[test]
    fn rejects_nested_repository_paths() {
        assert!(matches!(
            GitHubRepository::parse("https://github.com/example/project/tree/main"),
            Err(RepositoryError::InvalidGitHubUrl)
        ));
    }

    #[test]
    fn rejects_non_https_and_authenticated_urls() {
        for url in [
            "http://github.com/example/project",
            "ssh://git@github.com/example/project.git",
            "https://user@github.com/example/project",
        ] {
            assert!(matches!(
                GitHubRepository::parse(url),
                Err(RepositoryError::InvalidGitHubUrl)
            ));
        }
    }

    #[test]
    fn recognizes_supported_manifest_names() {
        for manifest in [
            "Cargo.toml",
            "package.json",
            "pyproject.toml",
            "go.mod",
            "project.csproj",
        ] {
            assert!(
                is_manifest_name(manifest),
                "{manifest} should be recognized"
            );
        }
    }

    #[test]
    fn skips_generated_and_dependency_directories() {
        assert!(should_skip_directory("node_modules"));
        assert!(should_skip_directory("target"));
        assert!(!should_skip_directory("src"));
    }
}
