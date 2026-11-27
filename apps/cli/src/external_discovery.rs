//! Read-only discovery of independently published Rust packages.
//! Reuses Cargo's existing crates.io search implementation instead of
//! implementing another registry client or regenerating third-party features.
//! Discovery is advisory; independent source/license review and the existing
//! verify-external-rust-crate gate remain mandatory.
use std::{error::Error, io, process::Command};

#[derive(Debug)]
struct Candidate {
    package: String,
    version: String,
    description: String,
    lexical_score: u32,
}

fn validate_request(query: &str, limit: usize) -> Result<(), Box<dyn Error>> {
    if !(1..=20).contains(&limit)
        || query.trim().is_empty()
        || query.len() > 160
        || query.split_whitespace().count() > 12
        || !query
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capability query must be 1-160 ASCII characters of words, spaces, dots, hyphens, or underscores; limit must be 1-20",
        )
        .into());
    }
    Ok(())
}

fn parse_candidates(output: &str, query: &str, limit: usize) -> Vec<Candidate> {
    let terms = query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let mut found = Vec::new();
    for line in output.lines().take(64) {
        let Some((package, details)) = line.split_once(" = \"") else {
            continue;
        };
        if package.is_empty()
            || package.len() > 64
            || !package.as_bytes()[0].is_ascii_alphabetic()
            || !package
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
        {
            continue;
        }
        let Some((version, suffix)) = details.split_once('"') else {
            continue;
        };
        if version.is_empty() || version.len() > 64 || semver::Version::parse(version).is_err() {
            continue;
        }
        let Some(description) = suffix.trim().strip_prefix('#') else {
            continue;
        };
        let description = description.trim().chars().take(500).collect::<String>();
        let name_lower = package.to_ascii_lowercase();
        let summary_lower = description.to_ascii_lowercase();
        let score = terms.iter().fold(0u32, |score, term| {
            score
                + if name_lower == *term {
                    70
                } else if name_lower.contains(term.as_str()) {
                    35
                } else if summary_lower.contains(term.as_str()) {
                    10
                } else {
                    0
                }
        });
        found.push(Candidate {
            package: package.to_owned(),
            version: version.to_owned(),
            description,
            lexical_score: score.min(100),
        });
    }
    found.sort_by(|a, b| {
        b.lexical_score
            .cmp(&a.lexical_score)
            .then(a.package.cmp(&b.package))
    });
    found.truncate(limit);
    found
}

/// Search crates.io through the existing Cargo executable, without
/// downloading or running candidate code and without touching any target.
/// The returned ranking is keyword overlap, NOT semantic compatibility.
pub(crate) fn discover_external_crates(
    query: &str,
    limit: usize,
) -> Result<serde_json::Value, Box<dyn Error>> {
    validate_request(query, limit)?;
    let search = Command::new("cargo")
        .arg("search")
        .arg("--limit")
        .arg(limit.to_string())
        .arg("--registry")
        .arg("crates-io")
        .arg("--")
        .arg(query)
        .env("CARGO_NET_RETRY", "0")
        .env("CARGO_HTTP_TIMEOUT", "15")
        .output()?;
    if !search.status.success() {
        let details = String::from_utf8_lossy(&search.stderr);
        return Err(io::Error::other(format!(
            "Cargo registry lookup failed: {}",
            details.chars().take(600).collect::<String>()
        ))
        .into());
    }
    if search.stdout.len() > 64 * 1024 {
        return Err(io::Error::other("Cargo search result exceeded 64 KiB").into());
    }
    let stdout = String::from_utf8(search.stdout)?;
    let candidates = parse_candidates(&stdout, query, limit)
        .into_iter()
        .map(|candidate| {
            serde_json::json!({
                "package": candidate.package,
                "published_version_unverified": candidate.version,
                "description": candidate.description,
                "lexical_score_not_confidence": candidate.lexical_score,
                "status": "requires_source_license_and_behavior_review",
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "schema_version": 1,
        "query": query,
        "registry": "crates.io through the installed Cargo search command",
        "reused_capability": "Cargo's existing registry discovery, not generated library implementations",
        "selection_status": "read_only_candidates_not_verified",
        "candidates": candidates,
        "next_step": "Check the upstream repository and license, pin a published version and commit, author target-owned tests, then run verify-external-rust-crate in restricted Docker",
        "warning": "Names and descriptions are untrusted registry data. Lexical ranking does not establish API, license, provenance, dependency or behavioral compatibility. Nothing is installed or integrated by this search.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_or_excessive_queries_without_invoking_cargo() {
        for bad in ["", " ", "jaro\nrm", "jaro;rm", "a\u{0}b"] {
            assert!(validate_request(bad, 5).is_err(), "{bad:?}");
        }
        assert!(validate_request("jaro similarity", 0).is_err());
        assert!(validate_request("jaro similarity", 21).is_err());
        assert!(validate_request("jaro similarity", 5).is_ok());
    }

    #[test]
    fn parses_and_ranks_registry_results_without_accepting_injected_lines() {
        let sample = "trigram = \"0.4.4\" # A trigram similarity implementation\n\
                      strsim = \"0.11.1\" # Jaro and Levenshtein string similarity\n\
                      garbage_name = \"xyz\" # malformed version\n\
                      bad;name = \"1.2.3\" # suspicious\n\
                      ... and 20 crates more\n";
        let found = parse_candidates(sample, "jaro similarity", 10);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].package, "strsim");
        assert_eq!(found[0].version, "0.11.1");
        assert!(found[0].lexical_score > found[1].lexical_score);
    }

    #[test]
    fn ignores_unrelated_output_and_respects_limit() {
        let found = parse_candidates(
            "alpha = \"1.2.3\" # support alpha\n\
             beta = \"1.2.3-rc.1\" # support beta\n\
             Gamma = \"1.2.3\" # invalid uppercase package\n",
            "support",
            1,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].package, "alpha");
    }
}
