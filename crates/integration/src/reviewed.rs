//! Fail-closed bridge from reviewed *suggestions* to compiler verification.
//! Never treats matching names as semantic proof. Explicit matching rule and
//! independently supplied target test are mandatory for every execution.
use std::{
    fs,
    path::{Component, Path},
};

use tree_sitter::Parser;

use crate::{
    VerifiedRustError,
    api_discovery::discover_rust_api_contracts,
    api_signature::{identifier, parse_rule},
};

const MAX_CONTRACT_TEST_BYTES: u64 = 128 * 1024;

/// Recompute the suggestion from the *current* source capsule and target.
/// A stale report, changed repository, ambiguous mapping, or missing contract
/// test is never permitted to initiate source execution.
pub(crate) fn validate_reviewed_api(
    capsule: &Path,
    target: &Path,
    rule: &str,
    contract_test: &Path,
) -> Result<(), VerifiedRustError> {
    let spec = parse_rule(rule)?;
    let report = discover_rust_api_contracts(capsule, target).map_err(|err| {
        VerifiedRustError::Unsafe(format!("API suggestion revalidation failed: {err}"))
    })?;
    let matches = report
        .requirements
        .iter()
        .filter(|requirement| requirement.target_function == spec.name)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(VerifiedRustError::Unsafe(
            "reviewed target function is missing or ambiguous".into(),
        ));
    }
    let best = matches[0].best_suggestion.as_ref().ok_or_else(|| {
        VerifiedRustError::Unsafe(
            "no unique, supported recommendation for reviewed API".into(),
        )
    })?;
    if best.suggested_cli_rule != rule {
        return Err(VerifiedRustError::Unsafe(
            "reviewed rule differs from freshly recomputed recommendation".into(),
        ));
    }
    validate_contract_test(target, contract_test, &spec.name)?;
    Ok(())
}

fn validate_contract_test(
    target: &Path,
    test: &Path,
    function: &str,
) -> Result<(), VerifiedRustError> {
    let components = test.components().collect::<Vec<_>>();
    let valid = matches!(
        components.as_slice(),
        [Component::Normal(folder), Component::Normal(filename)]
            if folder == &"tests" && Path::new(filename).extension().is_some_and(|e| e == "rs")
    );
    if !valid {
        return Err(VerifiedRustError::Unsafe(
            "reviewed contract test must be tests/<filename>.rs (no traversal)".into(),
        ));
    }
    let test_dir = target.join("tests");
    let dir = fs::symlink_metadata(&test_dir)?;
    if !dir.file_type().is_dir() || dir.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe("symlinked tests directory refused".into()));
    }
    let path = target.join(test);
    let meta = fs::symlink_metadata(&path)?;
    if !meta.file_type().is_file()
        || meta.len() == 0
        || meta.len() > MAX_CONTRACT_TEST_BYTES
    {
        return Err(VerifiedRustError::Unsafe(
            "contract test must be a regular, nonempty Rust file under 128 KiB".into(),
        ));
    }
    let code = fs::read_to_string(path)?;
    // An admission precheck, not a claim of complete coverage: Docker
    // runs Cargo tests and the reviewed caller must confirm meaningful
    // independent assertions. Never synthesize an acceptance test here.
    if !code.contains("#[test]")
        || !code.contains(&format!("{function}("))
        || !code.contains("assert")
        || code.contains("#[ignore]")
    {
        return Err(VerifiedRustError::Unsafe(
            "contract test must include active #[test], target call and assertion".into(),
        ));
    }
    Ok(())
}

/// Replace ONLY the body of a plain top-level public function that is still
/// exactly todo!()/unimplemented!(). Preserve its declaration and all tests;
/// never append a duplicate export or rewrite an existing implementation.
pub(crate) fn apply_reviewed_api(
    staged: &Path,
    module_name: &str,
    rule: &str,
) -> Result<String, VerifiedRustError> {
    let spec = parse_rule(rule)?;
    let lib = staged.join("src/lib.rs");
    let meta = fs::symlink_metadata(&lib)?;
    if !meta.file_type().is_file() || meta.len() > 1024 * 1024 {
        return Err(VerifiedRustError::Unsafe(
            "reviewed API target library must be a regular bounded file".into(),
        ));
    }
    let mut original = fs::read_to_string(&lib)?;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|err| VerifiedRustError::Unsafe(format!("Rust parser setup: {err}")))?;
    let tree = parser.parse(&original, None).ok_or_else(|| {
        VerifiedRustError::Unsafe("target library could not be parsed".into())
    })?;
    if tree.root_node().has_error() {
        return Err(VerifiedRustError::Unsafe(
            "target library contains Rust syntax errors".into(),
        ));
    }
    let mut cursor = tree.root_node().walk();
    let mut candidates = Vec::new();
    for node in tree.root_node().named_children(&mut cursor) {
        if node.kind() != "function_item" {
            continue;
        }
        let Some(name) = node.child_by_field_name("name") else {
            continue;
        };
        if name.utf8_text(original.as_bytes()).ok() != Some(spec.name.as_str()) {
            continue;
        }
        let args = node.child_by_field_name("parameters").ok_or_else(|| {
            VerifiedRustError::Unsafe("reviewed target has no parseable parameters".into())
        })?;
        let body = node.child_by_field_name("body").ok_or_else(|| {
            VerifiedRustError::Unsafe("reviewed target has no parseable body".into())
        })?;
        let prefix = original.get(node.start_byte()..name.start_byte())
            .ok_or_else(|| VerifiedRustError::Unsafe("invalid function span".into()))?;
        if prefix.trim() != "pub fn" {
            return Err(VerifiedRustError::Unsafe(
                "reviewed target is not a plain public function".into(),
            ));
        }
        let parameters = original
            .get(args.start_byte() + 1..args.end_byte().saturating_sub(1))
            .ok_or_else(|| VerifiedRustError::Unsafe("invalid parameter span".into()))?;
        let bindings = if parameters.trim().is_empty() {
            Vec::new()
        } else {
            parameters.split(',').map(|param| {
                let (name, ty) = param.trim().split_once(':').ok_or_else(|| {
                    VerifiedRustError::Unsafe("target parameters must be named scalar arguments".into())
                })?;
                let name = name.trim().strip_prefix("mut ").unwrap_or(name.trim());
                if !identifier(name) {
                    return Err(VerifiedRustError::Unsafe(
                        "target has unsupported parameter binding".into(),
                    ));
                }
                Ok((name.to_string(), ty.trim().to_string()))
            }).collect::<Result<Vec<_>, VerifiedRustError>>()?
        };
        if bindings.iter().map(|(_, ty)| ty).ne(spec.parameters.iter()) {
            return Err(VerifiedRustError::Unsafe(
                "reviewed target signature changed since discovery".into(),
            ));
        }
        let result = original
            .get(args.end_byte()..body.start_byte())
            .ok_or_else(|| VerifiedRustError::Unsafe("invalid return type span".into()))?
            .trim();
        let actual_return = result.strip_prefix("->").map(str::trim).unwrap_or("()");
        if actual_return != spec.result {
            return Err(VerifiedRustError::Unsafe(
                "reviewed target return type changed since discovery".into(),
            ));
        }
        let body_text = original.get(body.start_byte()..body.end_byte()).ok_or_else(|| {
            VerifiedRustError::Unsafe("invalid function body span".into())
        })?;
        let compact = body_text.chars().filter(|c| !c.is_whitespace())
            .collect::<String>();
        if !matches!(
            compact.as_str(),
            "{todo!()}" | "{todo!();}" | "{unimplemented!()}" | "{unimplemented!();}"
        ) {
            return Err(VerifiedRustError::Unsafe(
                "reviewed API no longer has an exact unfinished placeholder".into(),
            ));
        }
        let args = bindings
            .iter()
            .map(|(name, _)| format!("::core::convert::Into::into({name})"))
            .collect::<Vec<_>>().join(", ");
        let replacement = format!(
            "{{\n    // Composer: reviewed mapping, compiler-checked; behavior validated by target tests.\n\
                 let result: {} = crate::{module_name}::{}({args}).into();\n\
                 result\n}}",
            spec.result, spec.source,
        );
        candidates.push((body.start_byte(), body.end_byte(), replacement));
    }
    if candidates.len() != 1 {
        return Err(VerifiedRustError::Unsafe(
            "expected exactly one unfinished top-level public target function".into(),
        ));
    }
    let (start, end, replacement) = candidates.remove(0);
    original.replace_range(start..end, &replacement);
    fs::write(&lib, original)?;
    Ok(rule.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn substitutes_only_exact_stub_body() {
        let root = std::env::temp_dir().join(format!("composer-review-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/lib.rs");
        let before = "pub fn wanted(a: u32) -> u128 { todo!() }\n\
             pub fn existing(a: u32) -> u128 { a.into() }\n";
        fs::write(&file, before).unwrap();
        assert_eq!(
            apply_reviewed_api(&root, "composer_math", "euler_totient=wanted(u32)->u128")
                .unwrap(),
            "euler_totient=wanted(u32)->u128"
        );
        let after = fs::read_to_string(&file).unwrap();
        assert!(after.contains("crate::composer_math::euler_totient"));
        assert!(after.contains("pub fn existing(a: u32) -> u128 { a.into() }"));
        let snapshot = after.clone();
        assert!(apply_reviewed_api(&root, "composer_math", "euler_totient=wanted(u32)->u128").is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), snapshot);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_invalid_contract_file_paths_and_ignored_tests() {
        let root = std::env::temp_dir().join(format!("composer-contract-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("tests")).unwrap();
        let test = root.join("tests/contract.rs");
        fs::write(&test, "#[test] fn test() { assert_eq!(wanted(1), 1) }\n").unwrap();
        assert!(validate_contract_test(&root, Path::new("tests/contract.rs"), "wanted").is_ok());
        for path in ["../escape.rs", "tests/../../escape.rs", "src/lib.rs"] {
            assert!(validate_contract_test(&root, Path::new(path), "wanted").is_err());
        }
        fs::write(&test, "#[test]\n#[ignore]\nfn test() { assert_eq!(wanted(1), 1) }\n").unwrap();
        assert!(validate_contract_test(&root, Path::new("tests/contract.rs"), "wanted").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
