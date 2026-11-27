//! Type-only API bridge for independently reviewed upstream functions.
//! Adapted from Composer PR #20's tested borrowed-string adapter; it never
//! implements the capability itself. The upstream source stays unchanged.
use crate::{VerifiedRustError, api_signature::parse_rule};

/// Backwards-compatible reviewed adapter: without pinned source argument
/// metadata, only String -> &str conversions are accepted.
pub(crate) fn reviewed_string_adapter(
    crate_key: &str,
    source_symbol: &str,
    target_alias: &str,
    signature: Option<&str>,
) -> Result<Option<String>, VerifiedRustError> {
    reviewed_typed_git_adapter(crate_key, source_symbol, target_alias, signature, None)
}

/// A compiler-checked typed call to the *unchanged* external Git implementation.
/// Source argument types must come from Composer's trusted-location Tree-sitter
/// scan of the exact reviewed pinned checkout, never from an LLM or page text.
/// The only adaptation is String.as_str() for upstream &str; other scalars and
/// owned values are passed through unchanged, without casts or lossy coercions.
pub(crate) fn reviewed_typed_git_adapter(
    crate_key: &str,
    source_symbol: &str,
    target_alias: &str,
    signature: Option<&str>,
    source_parameters: Option<&[String]>,
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
    if spec.name != target_alias || spec.source != source_symbol {
        return Err(VerifiedRustError::Unsafe(
            "reviewed external adapter requires matching source symbol and target alias".into(),
        ));
    }

    let legacy = source_parameters.is_none();
    let source_params = if let Some(params) = source_parameters {
        if params.len() != spec.parameters.len() {
            return Err(VerifiedRustError::Unsafe(
                "inferred upstream parameter count differs from target signature".into(),
            ));
        }
        params.to_vec()
    } else {
        if spec.parameters.is_empty() || !spec.parameters.iter().all(|ty| ty == "String") {
            return Err(VerifiedRustError::Unsafe(
                "reviewed adapter requires target_alias(String[,String...])->supported_return_type"
                    .into(),
            ));
        }
        vec!["&str".to_owned(); spec.parameters.len()]
    };

    let compatible = spec
        .parameters
        .iter()
        .zip(&source_params)
        .all(|(target, source)| {
            (source == "&str" && target == "String")
                || (source != "&str" && source != "()" && target == source)
        });
    if !compatible {
        return Err(VerifiedRustError::Unsafe(
            "upstream and target parameter types require an unsupported conversion".into(),
        ));
    }
    if legacy && !source_params.iter().all(|param| param == "&str") {
        return Err(VerifiedRustError::Unsafe(
            "reviewed adapter must borrow target-owned Strings".into(),
        ));
    }

    let inputs = spec
        .parameters
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            let ty = if ty == "String" {
                "::std::string::String"
            } else {
                ty
            };
            format!("arg{index}: {ty}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let args = source_params
        .iter()
        .enumerate()
        .map(|(index, source)| {
            if source == "&str" {
                format!("arg{index}.as_str()")
            } else {
                format!("arg{index}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    // Only the compiler can decide whether the pinned upstream call typechecks.
    // The target's independent assertions determine exercised behavior.
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
    fn inferred_adapter_forwards_mixed_scalars_and_owned_string_without_casts() {
        let borrowed = reviewed_typed_git_adapter(
            "composer_git_strsim",
            "levenshtein",
            "distance",
            Some("distance(String,String)->usize"),
            Some(&["&str".to_owned(), "&str".to_owned()]),
        )
        .unwrap()
        .unwrap();
        assert!(borrowed.contains(
            "pub fn distance(arg0: ::std::string::String, arg1: ::std::string::String) -> usize"
        ));
        assert!(
            borrowed.contains("composer_git_strsim::levenshtein(arg0.as_str(), arg1.as_str())")
        );

        let mixed = reviewed_typed_git_adapter(
            "upstream",
            "limit",
            "bounded",
            Some("bounded(String,usize,bool)->bool"),
            Some(&["&str".to_owned(), "usize".to_owned(), "bool".to_owned()]),
        )
        .unwrap()
        .unwrap();
        assert!(mixed.contains("arg0: ::std::string::String, arg1: usize, arg2: bool"));
        assert!(mixed.contains("upstream::limit(arg0.as_str(), arg1, arg2)"));
        assert!(!mixed.contains(" as "));

        let by_value = reviewed_typed_git_adapter(
            "upstream",
            "shift",
            "moved",
            Some("moved(u32,i32)->i64"),
            Some(&["u32".to_owned(), "i32".to_owned()]),
        )
        .unwrap()
        .unwrap();
        assert!(by_value.contains("upstream::shift(arg0, arg1)"));

        for unsupported in [
            vec!["&str".to_owned(), "i32".to_owned()],
            vec!["usize".to_owned(), "usize".to_owned()],
            vec!["&mut str".to_owned(), "&str".to_owned()],
        ] {
            assert!(
                reviewed_typed_git_adapter(
                    "upstream",
                    "distance",
                    "mapped",
                    Some("mapped(String,String)->usize"),
                    Some(&unsupported),
                )
                .is_err()
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
