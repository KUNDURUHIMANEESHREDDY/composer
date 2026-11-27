//! Type-only API bridge for independently reviewed upstream functions.
//! Adapted from Composer PR #20's tested borrowed-string adapter; it never
//! implements the capability itself. The upstream source stays unchanged.
use crate::{VerifiedRustError, api_signature::parse_rule};

/// Borrow target-owned Strings for a reviewed upstream &str function.
/// Actual upstream signature and behavior are checked by Rust/Cargo and
/// target-owned independent contract tests inside the existing Docker verifier.
pub(crate) fn reviewed_string_adapter(
    crate_key: &str,
    source_symbol: &str,
    target_alias: &str,
    signature: Option<&str>,
) -> Result<Option<String>, VerifiedRustError> {
    let Some(signature) = signature else {
        return Ok(None);
    };
    if signature.len() > 180 {
        return Err(VerifiedRustError::Unsafe(
            "reviewed external API signature exceeds 180 bytes".into(),
        ));
    }
    let spec = parse_rule(&format!("{source_symbol}={signature}"))?;
    if spec.name != target_alias
        || spec.source != source_symbol
        || spec.parameters.is_empty()
        || !spec.parameters.iter().all(|ty| ty == "String")
    {
        return Err(VerifiedRustError::Unsafe(
            "reviewed adapter requires target_alias(String[,String...])->supported_return_type"
                .into(),
        ));
    }
    let inputs = spec
        .parameters
        .iter()
        .enumerate()
        .map(|(index, _)| format!("arg{index}: ::std::string::String"))
        .collect::<Vec<_>>()
        .join(", ");
    let args = spec
        .parameters
        .iter()
        .enumerate()
        .map(|(index, _)| format!("arg{index}.as_str()"))
        .collect::<Vec<_>>()
        .join(", ");
    // A compiler-checked single call to the existing upstream library.
    // No expressions, imports, user-supplied code, or algorithm synthesis.
    Ok(Some(format!(
        "\n// Composer: reviewed adapter to unchanged pinned Git capability\n\
         pub fn {}({inputs}) -> {} {{\n\
         {crate_key}::{}({args})\n\
         }}\n",
        spec.name, spec.result, source_symbol
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_string_adapter_preserves_upstream_scalar_result_types() {
        let jaro = reviewed_string_adapter(
            "composer_git_strsim",
            "jaro",
            "ported_jaro",
            Some("ported_jaro(String,String)->f64"),
        )
        .unwrap()
        .unwrap();
        assert!(jaro.contains(
            "pub fn ported_jaro(arg0: ::std::string::String, arg1: ::std::string::String) -> f64"
        ));
        assert!(jaro.contains("composer_git_strsim::jaro(arg0.as_str(), arg1.as_str())"));

        let predicate = reviewed_string_adapter(
            "upstream",
            "matches",
            "all_equal",
            Some("all_equal(String,String)->bool"),
        )
        .unwrap()
        .unwrap();
        assert!(predicate.contains("-> bool"));
        assert!(predicate.contains("matches(arg0.as_str(), arg1.as_str())"));

        let scalar = reviewed_string_adapter(
            "upstream",
            "length",
            "byte_length",
            Some("byte_length(String)->u64"),
        )
        .unwrap()
        .unwrap();
        assert!(scalar.contains("-> u64"));

        for invalid in [
            "ported_jaro(String,String)->&str",
            "ported_jaro(String,String)->Vec<u8>",
            "ported_jaro(String,String)->Result<f64,()>",
            "ported_jaro(&mut String,String)->f64",
            "ported_jaro(String,String)->f64;panic!()",
        ] {
            assert!(
                reviewed_string_adapter(
                    "composer_git_strsim",
                    "jaro",
                    "ported_jaro",
                    Some(invalid)
                )
                .is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn uses_existing_signature_parser_and_refuses_unreviewed_expressions() {
        let generated = reviewed_string_adapter(
            "composer_git_deunicode",
            "deunicode",
            "transliterate",
            Some("transliterate(String)->String"),
        )
        .unwrap()
        .unwrap();
        assert!(generated.contains("deunicode(arg0.as_str())"));
        assert!(generated.contains("pub fn transliterate(arg0: ::std::string::String)"));
        for invalid in [
            "transliterate(String);std::process::exit(1)->String",
            "transliterate(Vec<u8>)->String",
            "different(String)->String",
            "transliterate()->String",
            "transliterate(String)->&str",
            "transliterate(String)->String\npub fn evil() {}",
        ] {
            assert!(
                reviewed_string_adapter(
                    "composer_git_deunicode",
                    "deunicode",
                    "transliterate",
                    Some(invalid)
                )
                .is_err(),
                "{invalid}"
            );
        }
        assert!(
            reviewed_string_adapter("a", "b", "c", None)
                .unwrap()
                .is_none()
        );
    }
}
