//! Read-only discovery using Cargo's existing crates.io search/info capabilities
//! and Composer's existing static Rust analyzer. Never builds upstream source.
//!
//! Results are *candidates*, not approved or proven source-to-release matches.
//! The caller must still review code/licenses, then run independent contracts
//! through the existing offline Docker recovery verifier.
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use composer_intelligence::{SymbolKind, analyze_repository};
use composer_repository::GitHubRepository;
use serde::{Deserialize, Serialize};

use crate::{UpstreamRecoveryCandidate, VerifiedRustError};

const MAX_SEARCH_RESULTS: usize = 20;
const MAX_INFO_REQUESTS: usize = 12;
const MAX_CANDIDATES: usize = 5;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone)]
pub struct ExternalDiscoveryOptions {
    /// Query for Cargo's crates.io search (not raw GitHub site search).
    pub query: String,
    /// Required public Rust function name, e.g. jaro.
    pub function_name: String,
    /// Explicit owner exclusions, e.g. the user's own GitHub account.
    pub exclude_owners: Vec<String>,
    /// Pre-existing Cargo registry root, normally ~/.cargo/registry.
    pub registry_cache: PathBuf,
    pub max_candidates: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalDiscoveryResult {
    pub query: String,
    pub function_name: String,
    pub candidates: Vec<UpstreamRecoveryCandidate>,
    pub inspected_packages: usize,
    pub rejected_packages: usize,
    pub provenance_limit: String,
}

fn bounded_command(command: &mut Command, operation: &str) -> Result<String, VerifiedRustError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        VerifiedRustError::Dependency(format!("{operation}: executable unavailable: {error}"))
    })?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() >= COMMAND_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(VerifiedRustError::Dependency(format!(
                "{operation}: external metadata command timed out"
            )));
        }
        thread::sleep(Duration::from_millis(100));
    }
    let output = child.wait_with_output()?;
    if output.stdout.len() > 128 * 1024 || output.stderr.len() > 128 * 1024 {
        return Err(VerifiedRustError::Unsafe(format!(
            "{operation}: external metadata exceeds size limit"
        )));
    }
    if !output.status.success() {
        return Err(VerifiedRustError::Dependency(format!(
            "{operation}: metadata request failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(350)
                .collect::<String>()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| VerifiedRustError::Unsafe(format!("{operation}: non-UTF8 command response")))
}

fn parse_search_packages(response: &str) -> Vec<String> {
    let mut found = BTreeSet::new();
    let mut result = Vec::new();
    for line in response.lines() {
        // Cargo documents output as: name = "version" # description.
        let Some((name, _)) = line.split_once(" = \"") else {
            continue;
        };
        let name = name.trim();
        if name.is_empty()
            || name.len() > 64
            || !name.bytes().next().is_some_and(|c| c.is_ascii_alphabetic())
            || !name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
            || !found.insert(name.to_owned())
        {
            continue;
        }
        result.push(name.to_owned());
    }
    result
}

fn info_field<'a>(info: &'a str, name: &str) -> Option<&'a str> {
    info.lines().find_map(|line| {
        line.trim()
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix(':'))
            .map(str::trim)
    })
}

fn version_from_info(info: &str) -> Option<String> {
    let version = info_field(info, "version")?.split_whitespace().next()?;
    let parsed = semver::Version::parse(version).ok()?;
    if parsed.pre.is_empty() && parsed.to_string() == version {
        Some(version.into())
    } else {
        None
    }
}

fn public_symbol_from_sources(
    registry: &Path,
    package: &str,
    version: &str,
    function: &str,
) -> Option<String> {
    let source_roots = registry.join("src");
    let roots = fs::read_dir(source_roots).ok()?;
    let mut candidates = Vec::new();
    for root in roots.flatten() {
        let name = root.file_name().to_string_lossy().into_owned();
        if !name.starts_with("index.crates.io-") || !root.file_type().ok()?.is_dir() {
            continue;
        }
        let crate_root = root.path().join(format!("{package}-{version}"));
        let meta = match fs::symlink_metadata(&crate_root) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        candidates.push(crate_root);
    }
    // A bounded static parse of extracted registry source. Never executes it.
    for root in candidates.into_iter().take(4) {
        let lib_file = root.join("src/lib.rs");
        let Ok(lib) = fs::read_to_string(&lib_file) else {
            continue;
        };
        let Ok(graph) = analyze_repository(&root) else {
            continue;
        };
        let mut matches = Vec::new();
        for symbol in &graph.symbols {
            if symbol.name != function || symbol.kind != SymbolKind::Function {
                continue;
            }
            let path = symbol.file_path.as_path();
            let exported_name = if path == Path::new("src/lib.rs") {
                Some(function.to_string())
            } else if let Some(stem) = path.file_stem().and_then(|name| name.to_str()) {
                let module = if stem == "mod" {
                    path.parent()?.file_name()?.to_str()?
                } else {
                    stem
                };
                if path.parent() == Some(Path::new("src"))
                    || path.parent()?.parent() == Some(Path::new("src"))
                {
                    if lib.contains(&format!("pub mod {module};"))
                        || lib.contains(&format!("pub mod {module} {{"))
                    {
                        Some(format!("{module}::{function}"))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            let Some(exported_name) = exported_name else {
                continue;
            };
            let Ok(text) = fs::read_to_string(root.join(path)) else {
                continue;
            };
            let Some(fragment) = text.get(symbol.start_byte..symbol.end_byte) else {
                continue;
            };
            if fragment.trim_start().starts_with("pub fn ")
                || fragment.trim_start().starts_with("pub async fn ")
            {
                matches.push(exported_name);
            }
        }
        // A crate may expose the same function name in several public
        // modules (e.g. textdistance::str::jaro and ::nstr::jaro).
        // Prefer the root export, then its conventional str:: API.
        // This is only candidate ranking: independent tests remain mandatory.
        matches.sort_by_key(|path| {
            (
                path.matches("::").count(),
                !path.starts_with("str::"),
                path.len(),
            )
        });
        matches.dedup();
        if let Some(best) = matches.into_iter().next() {
            return Some(best);
        }
    }
    None
}

fn repository_head_sha(github: &GitHubRepository) -> Option<String> {
    // Running Git in the caller's checkout can load checkout-local URL
    // rewrites, even if global and environment-injected config is blocked.
    // Resolve public GitHub metadata from a fresh, empty directory instead.
    let scratch =
        std::env::temp_dir().join(format!("composer-git-discovery-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&scratch).ok()?;
    let mut command = isolated_repository_head_command(github, &scratch);
    let result = bounded_command(&mut command, "git ls-remote");
    // Always remove the empty temporary directory, even on metadata failures.
    let _ = fs::remove_dir(&scratch);
    let response = result.ok()?;
    let sha = response.split_whitespace().next()?;
    if sha.len() == 40 && sha.bytes().all(|value| value.is_ascii_hexdigit()) {
        Some(sha.to_owned())
    } else {
        None
    }
}

/// Use the existing hardened Git command with an isolated working directory.
/// A caller's checkout-local .git/config must never rewrite public URL lookups.
fn isolated_repository_head_command(github: &GitHubRepository, scratch: &Path) -> Command {
    let mut command = composer_repository::hardened_git_command();
    command
        .current_dir(scratch)
        // Do not ascend into a caller-controlled parent repository, even
        // when the TEMP directory itself is nested inside a Git checkout.
        .env(
            "GIT_CEILING_DIRECTORIES",
            scratch.parent().unwrap_or(scratch),
        )
        .args(["-c", "protocol.ext.allow=never", "ls-remote", "--"])
        .arg(&github.clone_url)
        .arg("HEAD");
    command
}

/// Search for existing capabilities by delegating to Cargo search/info.
/// The shortlist can be serialized directly into recover-external-rust-crate's
/// candidates JSON; it must still be independently reviewed before execution.
pub fn discover_external_rust(
    options: &ExternalDiscoveryOptions,
) -> Result<ExternalDiscoveryResult, VerifiedRustError> {
    if options.query.trim().len() < 2
        || options.query.len() > 64
        || !options
            .query
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b' '))
        || options.function_name.is_empty()
        || options.function_name.len() > 80
        || !options
            .function_name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        || !(1..=MAX_CANDIDATES).contains(&options.max_candidates)
        || options.exclude_owners.len() > 20
    {
        return Err(VerifiedRustError::Unsafe(
            "invalid capability query/function name or discovery bounds".into(),
        ));
    }
    let registry_meta = fs::symlink_metadata(&options.registry_cache)?;
    if !registry_meta.is_dir() || registry_meta.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe(
            "Cargo registry cache must be a regular directory".into(),
        ));
    }
    let mut search = Command::new("cargo");
    search
        .args(["search", options.query.trim(), "--limit"])
        .arg(MAX_SEARCH_RESULTS.to_string())
        .args(["--registry", "crates-io", "--color", "never"]);
    let hits = parse_search_packages(&bounded_command(&mut search, "cargo search")?);
    let mut candidates = Vec::new();
    let mut inspected = 0usize;
    let mut rejected = 0usize;
    let excluded = options
        .exclude_owners
        .iter()
        .map(|owner| owner.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut seen_repos = BTreeSet::new();
    for package in hits.into_iter().take(MAX_INFO_REQUESTS) {
        if candidates.len() >= options.max_candidates {
            break;
        }
        inspected += 1;
        let mut info = Command::new("cargo");
        info.args([
            "info",
            &package,
            "--registry",
            "crates-io",
            "--color",
            "never",
        ]);
        let details = match bounded_command(&mut info, "cargo info") {
            Ok(details) => details,
            Err(_) => {
                rejected += 1;
                continue;
            }
        };
        let Some(version) = version_from_info(&details) else {
            rejected += 1;
            continue;
        };
        let Some(repo) =
            info_field(&details, "repository").and_then(|url| GitHubRepository::parse(url).ok())
        else {
            rejected += 1;
            continue;
        };
        if excluded.contains(&repo.owner.to_ascii_lowercase())
            || !seen_repos.insert(repo.clone_url.clone())
            || info_field(&details, "license").is_none_or(|text| text == "unknown")
        {
            rejected += 1;
            continue;
        }
        let Some(public_symbol) = public_symbol_from_sources(
            &options.registry_cache,
            &package,
            &version,
            &options.function_name,
        ) else {
            rejected += 1;
            continue;
        };
        let Some(sha) = repository_head_sha(&repo) else {
            rejected += 1;
            continue;
        };
        candidates.push(UpstreamRecoveryCandidate {
            package,
            version_requirement: version,
            public_symbol,
            upstream_repository: repo.clone_url.trim_end_matches(".git").to_owned(),
            upstream_commit: sha,
        });
    }
    Ok(ExternalDiscoveryResult {
        query: options.query.clone(),
        function_name: options.function_name.clone(),
        candidates,
        inspected_packages: inspected,
        rejected_packages: rejected,
        provenance_limit: "Registry package version and extracted source were inspected without code execution. GitHub SHA is current HEAD, NOT a verified match to the published crates.io archive. Candidate selection requires source/license review and Docker behavioral tests.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_uses_isolated_git_metadata_command() {
        use std::ffi::OsStr;
        let github = GitHubRepository::parse("https://github.com/example/project").unwrap();
        let directory = std::env::temp_dir().join(format!(
            "composer-discovery-command-test-{}",
            uuid::Uuid::new_v4()
        ));
        let command = isolated_repository_head_command(&github, &directory);
        assert_eq!(command.get_current_dir(), Some(directory.as_path()));
        let args = command
            .get_args()
            .map(|item| item.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-c", "protocol.ext.allow=never"])
        );
        assert!(args.ends_with(&[
            "ls-remote".to_string(),
            "--".to_string(),
            "https://github.com/example/project.git".to_string(),
            "HEAD".to_string(),
        ]));
        let transport = command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("GIT_ALLOW_PROTOCOL"))
            .and_then(|(_, value)| value);
        assert_eq!(transport, Some(OsStr::new("https")));
        let ceiling = command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("GIT_CEILING_DIRECTORIES"))
            .and_then(|(_, value)| value);
        assert_eq!(ceiling, directory.parent().map(Path::as_os_str));
        for name in ["GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_DIR"] {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| { key == OsStr::new(name) && value.is_none() })
            );
        }
    }

    #[test]
    fn cargo_search_output_parses_exact_safe_names() {
        let output = "strsim = \"0.11.1\" # string similarity\ntextdistance = \"1.1.1\" # many algorithms\nstrsim = \"0.11.1\"\nnot safe = \"1.2.3\"\n";
        assert_eq!(parse_search_packages(output), ["strsim", "textdistance"]);
    }

    #[test]
    fn cargo_info_and_semver_are_pinned_without_latest_alias() {
        let info = "strsim #text\nversion: 0.11.1 (latest 0.12.0)\nlicense: MIT\nrepository: https://github.com/rapidfuzz/strsim-rs\n";
        assert_eq!(version_from_info(info).as_deref(), Some("0.11.1"));
        assert_eq!(info_field(info, "license"), Some("MIT"));
        assert!(version_from_info("version: ^0.11").is_none());
        assert!(version_from_info("version: 1.0.0-beta.2").is_none());
    }

    #[test]
    fn detects_public_api_path_using_existing_source_analyzer() {
        let dir = std::env::temp_dir().join(format!("composer-discovery-{}", uuid::Uuid::new_v4()));
        let crate_dir = dir.join("src/index.crates.io-test/example-1.2.3/src");
        fs::create_dir_all(&crate_dir).unwrap();
        fs::write(
            crate_dir.join("lib.rs"),
            "pub mod metrics;\npub fn direct(a: i32) -> i32 { a }\n// pub fn fake() {}\n",
        )
        .unwrap();
        fs::write(
            crate_dir.join("metrics.rs"),
            "pub fn jaro(a: &str, b: &str) -> f64 { if a == b {1.0} else {0.0} }\n",
        )
        .unwrap();
        assert_eq!(
            public_symbol_from_sources(&dir, "example", "1.2.3", "jaro").as_deref(),
            Some("metrics::jaro")
        );
        assert_eq!(
            public_symbol_from_sources(&dir, "example", "1.2.3", "direct").as_deref(),
            Some("direct")
        );
        assert!(public_symbol_from_sources(&dir, "example", "1.2.3", "fake").is_none());
        // Multiple legitimate public paths are ranked, never considered
        // semantic proof, and the conventional string API is preferred.
        fs::write(crate_dir.join("lib.rs"), "pub mod nstr;\npub mod str;\n").unwrap();
        fs::write(
            crate_dir.join("str.rs"),
            "pub fn jaro(a: &str, b: &str) -> f64 { if a == b {1.0} else {0.0} }\n",
        )
        .unwrap();
        fs::write(
            crate_dir.join("nstr.rs"),
            "pub fn jaro(a: &str, b: &str) -> f64 { if a == b {1.0} else {0.0} }\n",
        )
        .unwrap();
        assert_eq!(
            public_symbol_from_sources(&dir, "example", "1.2.3", "jaro").as_deref(),
            Some("str::jaro"),
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
