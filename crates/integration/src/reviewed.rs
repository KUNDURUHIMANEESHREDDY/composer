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
    // A type-compatible, name-unrelated source may be explicitly selected
    // by a reviewer, but is NEVER chosen automatically. Recompute it from
    // current, verified source and enforce the same independent test gate.
    let requirement = matches[0];
    let selected = requirement
        .candidates
        .iter()
        .find(|candidate| candidate.suggested_cli_rule == rule);
    if selected.is_none() {
        return Err(VerifiedRustError::Unsafe(
            "reviewed rule is absent from freshly recomputed type-compatible candidates".into(),
        ));
    }
    if let Some(best) = requirement.best_suggestion.as_ref()
        && best.suggested_cli_rule != rule
    {
        return Err(VerifiedRustError::Unsafe(
            "reviewed rule differs from unique high-confidence recommendation".into(),
        ));
    }
    validate_contract_test(target, contract_test, &spec.name)?;
    Ok(())
}

pub(crate) fn validate_contract_test(
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
    // Cargo can disable automatic integration-test discovery. Do not
    // accept a test file that the verification command may never execute.
    let cargo: toml::Value = toml::from_str(&fs::read_to_string(target.join("Cargo.toml"))?)
        .map_err(|err| VerifiedRustError::Toml(err.to_string()))?;
    let autotests_disabled = cargo
        .get("package")
        .and_then(|package| package.get("autotests"))
        .and_then(toml::Value::as_bool)
        == Some(false);
    if autotests_disabled {
        // When automatic discovery is disabled, Cargo runs only an explicitly
        // named test target. Require this precise file, the normal harness,
        // no required feature gates, and test execution enabled.
        let contract_name = test.file_stem().and_then(|s| s.to_str());
        let contract_path = test.to_str().map(|s| s.replace('\\', "/"));
        let declared = cargo
            .get("test")
            .and_then(toml::Value::as_array)
            .is_some_and(|targets| {
                targets.iter().any(|entry| {
                    entry.get("name").and_then(toml::Value::as_str) == contract_name
                        && entry.get("path").and_then(toml::Value::as_str)
                            == contract_path.as_deref()
                        && entry.get("harness").and_then(toml::Value::as_bool) != Some(false)
                        && entry.get("test").and_then(toml::Value::as_bool) != Some(false)
                        && entry.get("required-features").is_none()
                })
            });
        if !declared {
            return Err(VerifiedRustError::Unsafe(
                "autotests=false requires an exact, runnable explicit [[test]] registration".into(),
            ));
        }
    } else if let Some(configured) = cargo.get("test") {
        // With automatic test discovery enabled, existing unrelated [[test]]
        // entries (often feature-gated upstream integration tests) do not
        // replace our independently named tests/<contract>.rs. Reject any
        // collision by either name or path; otherwise preserve the target's
        // existing test configuration and require Docker cargo test success.
        let entries = configured.as_array().ok_or_else(|| {
            VerifiedRustError::Unsafe("Cargo [[test]] declarations are malformed".into())
        })?;
        let contract_name = test.file_stem().and_then(|s| s.to_str());
        let contract_path = test.to_str().map(|s| s.replace('\\', "/"));
        if entries.iter().any(|entry| {
            entry.get("name").and_then(toml::Value::as_str) == contract_name
                || entry.get("path").and_then(toml::Value::as_str) == contract_path.as_deref()
        }) {
            return Err(VerifiedRustError::Unsafe(
                "an explicit Cargo [[test]] target collides with the independent contract".into(),
            ));
        }
    }
    let test_dir = target.join("tests");
    let dir = fs::symlink_metadata(&test_dir)?;
    if !dir.file_type().is_dir() || dir.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe(
            "symlinked tests directory refused".into(),
        ));
    }
    let path = target.join(test);
    let meta = fs::symlink_metadata(&path)?;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > MAX_CONTRACT_TEST_BYTES {
        return Err(VerifiedRustError::Unsafe(
            "contract test must be a regular, nonempty Rust file under 128 KiB".into(),
        ));
    }
    let code = fs::read_to_string(path)?;
    // Parse test syntax, rather than accepting tokens embedded in comments
    // or string literals. This is still an admission check, not proof of
    // semantic correctness; callers must independently review assertions.
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|err| VerifiedRustError::Unsafe(format!("Rust test parser: {err}")))?;
    let tree = parser
        .parse(&code, None)
        .ok_or_else(|| VerifiedRustError::Unsafe("contract test could not be parsed".into()))?;
    if tree.root_node().has_error() {
        return Err(VerifiedRustError::Unsafe(
            "contract test has invalid Rust syntax".into(),
        ));
    }
    if contains_disabling_attributes(tree.root_node(), &code) {
        return Err(VerifiedRustError::Unsafe(
            "conditional, ignored or expected-panic test attributes are not permitted".into(),
        ));
    }
    let mut attrs = Vec::new();
    let mut valid_test = false;
    let mut cursor = tree.root_node().walk();
    for item in tree.root_node().named_children(&mut cursor) {
        if item.kind() == "attribute_item" {
            let attr = item
                .utf8_text(code.as_bytes())
                .map_err(|_| VerifiedRustError::Unsafe("invalid test attribute".into()))?;
            let compact = attr
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            attrs.push(compact);
            continue;
        }
        if item.kind() == "inner_attribute_item" {
            return Err(VerifiedRustError::Unsafe(
                "crate-level attributes in contract tests require manual review".into(),
            ));
        }
        if item.kind() == "function_item"
            && attrs.iter().any(|a| a == "#[test]")
            && item
                .child_by_field_name("body")
                .is_some_and(|body| has_target_assertion(body, &code, function))
        {
            valid_test = true;
        }
        attrs.clear();
    }
    if !valid_test {
        return Err(VerifiedRustError::Unsafe(
            "contract test must have an active top-level #[test] with an assertion invoking the target API".into(),
        ));
    }
    Ok(())
}

fn contains_disabling_attributes(node: tree_sitter::Node<'_>, code: &str) -> bool {
    if matches!(node.kind(), "attribute_item" | "inner_attribute_item") {
        let compact = node
            .utf8_text(code.as_bytes())
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        if [
            "#[ignore",
            "#[cfg(",
            "#[cfg_attr(",
            "#[should_panic",
            "#![cfg(",
            "#![cfg_attr(",
        ]
        .iter()
        .any(|prefix| compact.starts_with(prefix))
        {
            return true;
        }
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| contains_disabling_attributes(child, code))
}

// Require a direct invocation as the first assertion argument. A target
// name nested in stringify!, a closure that never executes, or another macro
// must not make a vacuous assertion look like a behavioral contract. This
// deliberately rejects more complex expressions pending manual review.
fn has_target_assertion(node: tree_sitter::Node<'_>, code: &str, function: &str) -> bool {
    // A nested assertion may never execute (uncalled helper, conditional
    // branch or empty loop). Demand at least one simple unconditional
    // assertion instead; more complex test structure needs manual review.
    if matches!(
        node.kind(),
        "closure_expression"
            | "function_item"
            | "async_block"
            | "async_block_expression"
            | "if_expression"
            | "match_expression"
            | "for_expression"
            | "while_expression"
            | "loop_expression"
    ) {
        return false;
    }
    if node.kind() == "macro_invocation" {
        let macro_name = node
            .child_by_field_name("macro")
            .and_then(|name| name.utf8_text(code.as_bytes()).ok());
        if matches!(macro_name, Some("assert" | "assert_eq" | "assert_ne")) {
            let mut cursor = node.walk();
            let arguments = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "token_tree");
            if let Some(text) = arguments.and_then(|a| a.utf8_text(code.as_bytes()).ok())
                && let Some(body) = text.strip_prefix('(')
            {
                let compact = body
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>();
                if (compact.starts_with(&format!("{function}("))
                    || compact.starts_with(&format!("crate::{function}(")))
                    && compact.matches(&format!("{function}(")).count() == 1
                {
                    return true;
                }
            }
        }
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| has_target_assertion(child, code, function))
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
    let tree = parser
        .parse(&original, None)
        .ok_or_else(|| VerifiedRustError::Unsafe("target library could not be parsed".into()))?;
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
        let prefix = original
            .get(node.start_byte()..name.start_byte())
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
            parameters
                .split(',')
                .map(|param| {
                    let (name, ty) = param.trim().split_once(':').ok_or_else(|| {
                        VerifiedRustError::Unsafe(
                            "target parameters must be named scalar arguments".into(),
                        )
                    })?;
                    let name = name.trim().strip_prefix("mut ").unwrap_or(name.trim());
                    if !identifier(name) {
                        return Err(VerifiedRustError::Unsafe(
                            "target has unsupported parameter binding".into(),
                        ));
                    }
                    Ok((name.to_string(), ty.trim().to_string()))
                })
                .collect::<Result<Vec<_>, VerifiedRustError>>()?
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
        let body_text = original
            .get(body.start_byte()..body.end_byte())
            .ok_or_else(|| VerifiedRustError::Unsafe("invalid function body span".into()))?;
        let compact = body_text
            .chars()
            .filter(|c| !c.is_whitespace())
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
            .collect::<Vec<_>>()
            .join(", ");
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
    fn heldout30_shared_behavioral_contract_passes_admission_without_weakening_policy() {
        // Exercise the actual Python-generated fixture used for all 30 CI targets.
        // Its unconditional assertion must pass the strict parser, while the
        // earlier loop-only assertion remains disallowed.
        let harness = include_str!("../../../tests/test_heldout30_functional_anyhow.py");
        let oracle = harness
            .split("INDEPENDENT_ORACLE = \"\"\"")
            .nth(1)
            .and_then(|text| text.split("\"\"\"").next())
            .expect("heldout30 Python fixture must contain the shared oracle");
        let root =
            std::env::temp_dir().join(format!("composer-heldout30-admission-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("tests")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='heldout-contract-check'\nversion='0.1.0'\n",
        )
        .unwrap();
        let contract = root.join("tests/heldout.rs");
        fs::write(&contract, oracle).unwrap();
        assert!(
            validate_contract_test(&root, Path::new("tests/heldout.rs"), "ported_totient").is_ok(),
            "shared held-out functional contract must pass strict AST admission"
        );
        let previous = oracle.replacen("    assert_eq!(ported_totient(36), 12);\n", "", 1);
        assert_ne!(
            previous, oracle,
            "unconditional edge-case assertion disappeared"
        );
        fs::write(&contract, previous).unwrap();
        assert!(
            validate_contract_test(&root, Path::new("tests/heldout.rs"), "ported_totient").is_err(),
            "loop-only assertions must still be rejected"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn substitutes_only_exact_stub_body() {
        let root = std::env::temp_dir().join(format!("composer-review-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/lib.rs");
        let before = "pub fn wanted(a: u32) -> u128 { todo!() }\n\
             pub fn existing(a: u32) -> u128 { a.into() }\n";
        fs::write(&file, before).unwrap();
        assert_eq!(
            apply_reviewed_api(&root, "composer_math", "euler_totient=wanted(u32)->u128").unwrap(),
            "euler_totient=wanted(u32)->u128"
        );
        let after = fs::read_to_string(&file).unwrap();
        assert!(after.contains("crate::composer_math::euler_totient"));
        assert!(after.contains("pub fn existing(a: u32) -> u128 { a.into() }"));
        let snapshot = after.clone();
        assert!(
            apply_reviewed_api(&root, "composer_math", "euler_totient=wanted(u32)->u128").is_err()
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), snapshot);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_contract_spoofing_and_nonexecuting_attributes() {
        let root = std::env::temp_dir().join(format!("composer-contract-ast-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("tests")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .unwrap();
        let test = root.join("tests/reviewed.rs");
        let valid = "#[test] fn checks() { assert_eq!(wanted(1), 2); }";
        for accepted in [
            valid,
            "#[test] fn checks() { assert!(wanted(1)); }",
            "#[test] fn checks() { assert_ne!(wanted(1), 0); }",
            "#[test] fn checks() { assert_eq!(crate::wanted(1), 2); }",
        ] {
            fs::write(&test, accepted).unwrap();
            assert!(
                validate_contract_test(&root, Path::new("tests/reviewed.rs"), "wanted").is_ok(),
                "valid checked expression rejected: {accepted}"
            );
        }
        for bogus in [
            "/* #[test] wanted(1) assert */ fn unrelated() {}",
            "#[test] fn unrelated() { let text = \"wanted(1)\"; assert_eq!(1, 1); }",
            "#[test] fn unrelated() { assert_eq!(1, 1); } // wanted(1)",
            "#[test] fn unrelated() { let _ = wanted(1); assert_eq!(1, 1); }",
            "#[test] fn unrelated() { assert_eq!(other.wanted(1), 2); }",
            "#[test] fn unrelated() { assert_eq!(stringify!(wanted(1)), \"wanted(1)\"); }",
            "#[test] fn unrelated() { assert_eq!(1, 1); let f = || wanted(1); }",
            "#[test] fn unrelated() { assert_eq!(1, wanted(1)); }",
            "#[test] fn unrelated() { assert_eq!(wanted(1), wanted(1)); }",
            "#[test] fn unrelated() { assert_eq!(wanted (1), wanted (1)); }",
            "#[test] fn unrelated() { let unused = || assert_eq!(wanted(1), 2); }",
            "#[test] fn unrelated() { fn unused() { assert_eq!(wanted(1), 2); } }",
            "#[test] fn unrelated() { if false { assert_eq!(wanted(1), 2); } }",
            "#[test] fn unrelated() { for _ in 0..0 { assert_eq!(wanted(1), 2); } }",
            "#[test] fn unrelated() { while false { assert_eq!(wanted(1), 2); } }",
            "#[test] fn unrelated() { match 0 { 1 => assert_eq!(wanted(1), 2), _ => {} } }",
            "#[test] #[should_panic] fn check() { assert_eq!(wanted(1), 2); }",
            "#[test] #[cfg(any())] fn check() { assert_eq!(wanted(1), 2); }",
            "#[ignore] #[test] fn check() { assert_eq!(wanted(1), 2); }",
            "#[test] fn check() { #[cfg(any())] assert_eq!(wanted(1), 2); }",
        ] {
            fs::write(&test, bogus).unwrap();
            assert!(
                validate_contract_test(&root, Path::new("tests/reviewed.rs"), "wanted").is_err(),
                "incorrectly admitted: {bogus}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_invalid_contract_file_paths_and_ignored_tests() {
        let root = std::env::temp_dir().join(format!("composer-contract-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("tests")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .unwrap();
        let test = root.join("tests/contract.rs");
        fs::write(&test, "#[test] fn test() { assert_eq!(wanted(1), 1) }\n").unwrap();
        assert!(validate_contract_test(&root, Path::new("tests/contract.rs"), "wanted").is_ok());
        let original_manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\nautotests=false\n",
        )
        .unwrap();
        assert!(validate_contract_test(&root, Path::new("tests/contract.rs"), "wanted").is_err());
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\nautotests=false\n[[test]]\nname='contract'\npath='tests/contract.rs'\n",
        ).unwrap();
        assert!(validate_contract_test(&root, Path::new("tests/contract.rs"), "wanted").is_ok());
        // Exact registration is allowed only when Cargo's discovery is off
        // and none of the registration flags can skip the independent contract.
        for blocked in [
            "harness=false\n",
            "test=false\n",
            "required-features=['disabled']\n",
        ] {
            fs::write(
                root.join("Cargo.toml"),
                format!(
                    "[package]\nname='demo'\nversion='0.1.0'\nautotests=false\n[[test]]\nname='contract'\npath='tests/contract.rs'\n{blocked}"
                ),
            )
            .unwrap();
            assert!(
                validate_contract_test(&root, Path::new("tests/contract.rs"), "wanted").is_err(),
                "non-executable explicit test was admitted: {blocked}"
            );
        }
        fs::write(root.join("Cargo.toml"), original_manifest.clone()).unwrap();
        // Existing upstream [[test]] entries must not mask the new
        // automatically discovered contract, but unrelated tests are safe.
        for unrelated in [
            "[[test]]\nname='upstream'\npath='tests/upstream.rs'\n",
            "[[test]]\nname='upstream'\npath='tests/upstream.rs'\nrequired-features=['full']\n",
        ] {
            fs::write(
                root.join("Cargo.toml"),
                format!("{original_manifest}{unrelated}"),
            )
            .unwrap();
            assert!(
                validate_contract_test(&root, Path::new("tests/contract.rs"), "wanted").is_ok(),
                "unrelated upstream test must not block independent contract: {unrelated}"
            );
        }
        for explicit in [
            "[[test]]\nname='contract'\npath='tests/contract.rs'\nharness=false\n",
            "[[test]]\nname='contract'\npath='tests/contract.rs'\nrequired-features=['optional']\n",
            "[[test]]\nname='contract'\npath='tests/other.rs'\n",
            "[[test]]\nname='unrelated'\npath='tests/contract.rs'\n",
        ] {
            fs::write(
                root.join("Cargo.toml"),
                format!("{original_manifest}{explicit}"),
            )
            .unwrap();
            assert!(
                validate_contract_test(&root, Path::new("tests/contract.rs"), "wanted").is_err(),
                "explicit Cargo test configuration was accepted: {explicit}"
            );
        }
        fs::write(root.join("Cargo.toml"), original_manifest).unwrap();
        for path in ["../escape.rs", "tests/../../escape.rs", "src/lib.rs"] {
            assert!(validate_contract_test(&root, Path::new(path), "wanted").is_err());
        }
        fs::write(
            &test,
            "#[test]\n#[ignore]\nfn test() { assert_eq!(wanted(1), 1) }\n",
        )
        .unwrap();
        assert!(validate_contract_test(&root, Path::new("tests/contract.rs"), "wanted").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
