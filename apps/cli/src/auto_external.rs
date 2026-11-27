//! Bounded automatic package selection from human-reviewed external sources.
//! Only the orchestration/adapter is new: Cargo and the published upstream
//! crate supply the implementation. There is no arbitrary code generation.
use std::{error::Error, fs, io, path::PathBuf, process::Command};

use composer_integration::{UpstreamCrateOptions, verify_upstream_crate};
use serde_json::{Value, json};

use crate::external_discovery::discover_external_crates;

#[derive(Debug, Clone)]
struct ApprovedSource {
    package: String,
    version: String,
    public_symbol: String,
    repository: String,
    commit: String,
    license: String,
}

pub(crate) struct AutoExternalOptions {
    pub capability: String,
    pub catalog: PathBuf,
    pub target: PathBuf,
    pub output: PathBuf,
    pub target_alias: String,
    pub contract_test: PathBuf,
    pub registry_cache: PathBuf,
    pub excluded_owners: Vec<String>,
    pub license_reviewed: bool,
}

fn invalid(message: &str) -> Box<dyn Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message))
}

fn text_field<'a>(object: &'a Value, key: &str) -> Result<&'a str, Box<dyn Error>> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("approved catalog has a missing or empty string field"))
}

fn approved_source(
    catalog: &Value,
    capability: &str,
    excluded_owners: &[String],
) -> Result<ApprovedSource, Box<dyn Error>> {
    let request = capability.trim().to_ascii_lowercase();
    if request.is_empty()
        || request.len() > 120
        || !request
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
    {
        return Err(invalid(
            "capability name must be a short, plain-text identifier",
        ));
    }
    let records = catalog
        .get("approved_sources")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= 32)
        .ok_or_else(|| invalid("catalog must contain 1-32 approved_sources"))?;
    if catalog.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(invalid("unsupported approved source catalog schema"));
    }

    let mut selected = Vec::new();
    for item in records {
        let package = text_field(item, "package")?;
        let version = text_field(item, "version")?;
        let public_symbol = text_field(item, "public_symbol")?;
        let repository = text_field(item, "repository")?;
        let commit = text_field(item, "commit")?;
        let license = text_field(item, "license")?;
        let aliases = item
            .get("capabilities")
            .and_then(Value::as_array)
            .filter(|values| !values.is_empty() && values.len() <= 12)
            .ok_or_else(|| invalid("approved source must specify 1-12 capability names"))?;

        if package.len() > 64
            || !package.as_bytes()[0].is_ascii_lowercase()
            || !package
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
            || semver::Version::parse(version).is_err()
            || version.len() > 64
            || public_symbol.len() > 80
            || !public_symbol.as_bytes()[0].is_ascii_alphabetic()
            || !public_symbol
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || commit.len() != 40
            || !commit.bytes().all(|b| b.is_ascii_hexdigit())
            || license.len() > 100
        {
            return Err(invalid(
                "approved source contains invalid package, exact version, symbol, commit, or license",
            ));
        }

        let Some(parts) = repository.strip_prefix("https://github.com/") else {
            return Err(invalid("upstream must be a public GitHub repository URL"));
        };
        let (owner, name) = parts
            .split_once('/')
            .ok_or_else(|| invalid("upstream repository must contain owner and name"))?;
        if owner.is_empty()
            || name.is_empty()
            || name.contains('/')
            || !owner
                .bytes()
                .chain(name.bytes())
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(invalid("invalid upstream repository location"));
        }
        if excluded_owners
            .iter()
            .any(|excluded| owner.eq_ignore_ascii_case(excluded))
        {
            return Err(invalid(
                "approved source is owned by an excluded repository owner",
            ));
        }
        let mut matches = false;
        for alias in aliases {
            let alias = alias
                .as_str()
                .filter(|name| !name.trim().is_empty() && name.len() <= 120)
                .ok_or_else(|| invalid("invalid catalog capability alias"))?;
            if alias.trim().eq_ignore_ascii_case(&request) {
                matches = true;
            }
        }
        if matches {
            selected.push(ApprovedSource {
                package: package.to_owned(),
                version: version.to_owned(),
                public_symbol: public_symbol.to_owned(),
                repository: repository.to_owned(),
                commit: commit.to_owned(),
                license: license.to_owned(),
            });
        }
    }
    if selected.len() != 1 {
        return Err(invalid(
            "no unique reviewed source for capability; refuse unknown or ambiguous matches",
        ));
    }
    Ok(selected.remove(0))
}

/// Cargo's registry metadata is external data: check both the *exact*
/// package and version, not a fuzzy search result or description.
fn cargo_info_matches(info: &str, package: &str, version: &str) -> bool {
    let header = info.lines().find(|line| !line.trim().is_empty());
    let named = header.and_then(|line| line.split_whitespace().next()) == Some(package);
    let versioned = info.lines().any(|line| {
        line.trim()
            .strip_prefix("version:")
            .and_then(|value| value.split_whitespace().next())
            == Some(version)
    });
    named && versioned
}

/// Select a pinned, reviewed third-party package, confirm registry presence,
/// and use the existing fail-closed Docker verifier. This is NOT general AI
/// selection: no source enters execution from search results alone.
pub(crate) fn compose_approved_external(
    options: &AutoExternalOptions,
) -> Result<Value, Box<dyn Error>> {
    // Do not run network lookups, read untrusted source code or mutate a
    // target without explicit, separate source/license review.
    if !options.license_reviewed {
        return Err(invalid(
            "independent source/license review required before composition",
        ));
    }
    let catalog_info = fs::symlink_metadata(&options.catalog)?;
    if !catalog_info.is_file()
        || catalog_info.file_type().is_symlink()
        || catalog_info.len() > 64 * 1024
    {
        return Err(invalid(
            "catalog must be a regular JSON file no larger than 64 KiB",
        ));
    }
    let catalog: Value = serde_json::from_slice(&fs::read(&options.catalog)?)?;
    let selected = approved_source(&catalog, &options.capability, &options.excluded_owners)?;

    // Registry search is provided by Cargo and is read-only. Search its
    // exact known package name, never trust description text as instructions.
    let discovered = discover_external_crates(&selected.package, 20)?;
    let candidates = discovered
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Cargo registry search did not return candidate records"))?;
    let registry_matches = candidates
        .iter()
        .filter(|entry| {
            entry.get("package").and_then(Value::as_str) == Some(selected.package.as_str())
        })
        .collect::<Vec<_>>();
    if registry_matches.len() > 1 {
        return Err(invalid("ambiguous crate identity in crates.io search"));
    }

    // Cargo search is an advisory keyword index: its free-text output is
    // not guaranteed to list every published crate. Reuse Cargo's exact
    // package/version metadata rather than trusting a parser's absence.
    // Cargo info may download the pinned .crate archive but never executes it.
    let info = Command::new("cargo")
        .arg("info")
        .arg(format!("{}@{}", selected.package, selected.version))
        .arg("--registry")
        .arg("crates-io")
        .env("CARGO_NET_RETRY", "0")
        .env("CARGO_HTTP_TIMEOUT", "15")
        .output()?;
    if !info.status.success() || info.stdout.len() > 16 * 1024 {
        return Err(invalid(
            "Cargo could not verify the approved exact package version",
        ));
    }
    let info_output = String::from_utf8(info.stdout)?;
    if !cargo_info_matches(&info_output, &selected.package, &selected.version) {
        return Err(invalid(
            "Cargo registry package identity or pinned version mismatch",
        ));
    }
    let latest_seen = registry_matches
        .first()
        .and_then(|candidate| candidate.get("published_version_unverified"))
        .and_then(Value::as_str)
        .unwrap_or("not_listed_by_keyword_search");

    // Exact package version, independently supplied behavioral contract and
    // immutable provenance claim pass into the existing verifier unchanged.
    // It refuses unsafe Cargo conflicts and publishes ONLY on test success.
    let verified = verify_upstream_crate(&UpstreamCrateOptions {
        target_root: options.target.clone(),
        output_root: options.output.clone(),
        package: selected.package.clone(),
        version: selected.version.clone(),
        public_symbol: selected.public_symbol.clone(),
        target_alias: options.target_alias.clone(),
        contract_test: options.contract_test.clone(),
        registry_cache: options.registry_cache.clone(),
        upstream_repository: selected.repository.clone(),
        upstream_commit: selected.commit.clone(),
        license_reviewed: options.license_reviewed,
    })?;
    Ok(json!({
        "schema_version": 1,
        "mode": "auto_select_reviewed_external_package_then_verify",
        "selection": {
            "source": "uniquely matching reviewed catalog entry confirmed by Cargo registry search",
            "capability": options.capability,
            "package": selected.package,
            "approved_pinned_version": selected.version,
            "registry_latest_version_unverified": latest_seen,
            "public_symbol": selected.public_symbol,
            "repository": selected.repository,
            "commit_claimed": selected.commit,
            "license_declared_by_reviewed_catalog": selected.license,
        },
        "integration": verified,
        "selection_limitations": "Approved-catalog matching is exact and bounded, not arbitrary open-world discovery. Registry metadata is untrusted. Git SHA is not cryptographically matched to the published crate archive. Independent tests cover only authored cases.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Value {
        json!({
            "schema_version": 1,
            "approved_sources": [{
                "package": "strsim",
                "version": "0.11.1",
                "public_symbol": "jaro",
                "repository": "https://github.com/rapidfuzz/strsim-rs",
                "commit": "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26",
                "license": "MIT",
                "capabilities": ["jaro", "jaro string similarity"]
            }]
        })
    }

    #[test]
    fn chooses_external_approved_package_without_inventing_new_code() {
        let selected = approved_source(&catalog(), "JARO string similarity", &[]).unwrap();
        assert_eq!(selected.package, "strsim");
        assert_eq!(selected.version, "0.11.1");
        assert_eq!(selected.public_symbol, "jaro");
        assert!(approved_source(&catalog(), "jaro-winkler", &[]).is_err());
    }

    #[test]
    fn rejects_ambiguity_and_self_owned_repositories() {
        let mut data = catalog();
        let duplicate = data["approved_sources"][0].clone();
        data["approved_sources"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(approved_source(&data, "jaro", &[]).is_err());
        assert!(approved_source(&catalog(), "jaro", &["rapidfuzz".to_owned()]).is_err());
    }

    #[test]
    fn rejects_invalid_versions_and_unreviewed_catalogs() {
        let mut data = catalog();
        data["approved_sources"][0]["version"] = json!("^0.11");
        assert!(approved_source(&data, "jaro", &[]).is_err());
        data["approved_sources"][0]["version"] = json!("0.11.1");
        data["approved_sources"][0]["repository"] =
            json!("https://github.com/rapidfuzz/strsim-rs/tree/main");
        assert!(approved_source(&data, "jaro", &[]).is_err());
        assert!(approved_source(&catalog(), "jaro; run code", &[]).is_err());
    }

    #[test]
    fn exact_cargo_metadata_does_not_accept_name_or_version_confusion() {
        let info =
            "strsim #jaro #similarity\nString similarity metrics\nversion: 0.11.1\nlicense: MIT\n";
        assert!(cargo_info_matches(info, "strsim", "0.11.1"));
        assert!(!cargo_info_matches(info, "strsim", "0.10.0"));
        assert!(!cargo_info_matches(info, "strsim-copy", "0.11.1"));
        assert!(!cargo_info_matches(
            "strsim\nversion: 0.10.0\n",
            "strsim",
            "0.11.1"
        ));
    }

    #[test]
    fn review_gate_precedes_filesystem_and_network_access() {
        let options = AutoExternalOptions {
            capability: "jaro".to_owned(),
            catalog: PathBuf::from("no-such-catalog"),
            target: PathBuf::from("no-such-target"),
            output: PathBuf::from("no-such-output"),
            target_alias: "ported_jaro".to_owned(),
            contract_test: PathBuf::from("tests/contract.rs"),
            registry_cache: PathBuf::from("no-cache"),
            excluded_owners: Vec::new(),
            license_reviewed: false,
        };
        assert!(compose_approved_external(&options).is_err());
    }
}
