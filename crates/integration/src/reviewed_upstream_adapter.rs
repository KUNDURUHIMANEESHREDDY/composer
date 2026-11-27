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
        assert!(reviewed_string_adapter("a", "b", "c", None).unwrap().is_none());
    }
}
