//! Explicit target-facing signature contracts with compiler-inferred conversions.
//! Only infallible Into/From implementations can satisfy the generated wrapper.
//! Compiler tests remain the actual acceptance gate; no user Rust expressions
//! or arbitrary snippets are ever accepted into this generator.
use std::{collections::BTreeSet, fs, path::Path};
#[cfg(test)]
use uuid::Uuid;

use crate::VerifiedRustError;

const MAX_ADAPTERS: usize = 8;
const MAX_PARAMETERS: usize = 4;
const MAX_RULE_BYTES: usize = 200;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Signature {
    pub(crate) source: String,
    pub(crate) name: String,
    pub(crate) parameters: Vec<String>,
    pub(crate) result: String,
}

pub(crate) fn identifier(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && s != "_"
        && b.iter().all(|x| x.is_ascii_alphanumeric() || *x == b'_')
}

fn allowed_type(s: &str, is_result: bool) -> bool {
    matches!(
        s,
        "u8" | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
            | "char"
            | "bool"
            | "String"
    ) || (!is_result && s == "&str")
        || (is_result && s == "()")
}

pub(crate) fn parse_rule(input: &str) -> Result<Signature, VerifiedRustError> {
    let bad = || {
        VerifiedRustError::Unsafe(format!(
            "unsupported API signature: {input}; expected source::item=public_fn(u16)->u64"
        ))
    };
    if input.is_empty() || input.len() > MAX_RULE_BYTES {
        return Err(bad());
    }
    let (source, target) = input.split_once('=').ok_or_else(bad)?;
    let paths = source.split("::").collect::<Vec<_>>();
    if paths.is_empty() || paths.len() > 8 || !paths.iter().all(|v| identifier(v)) {
        return Err(bad());
    }
    let (header, result) = target.rsplit_once("->").ok_or_else(bad)?;
    let (name, args) = header.split_once('(').ok_or_else(bad)?;
    let args = args.strip_suffix(')').ok_or_else(bad)?;
    let result = result.trim();
    if !identifier(name) || !allowed_type(result, true) {
        return Err(bad());
    }
    let parameters = if args.trim().is_empty() {
        Vec::new()
    } else {
        args.split(',')
            .map(|s| s.trim().to_owned())
            .collect::<Vec<_>>()
    };
    if parameters.len() > MAX_PARAMETERS || !parameters.iter().all(|s| allowed_type(s, false)) {
        return Err(bad());
    }
    Ok(Signature {
        source: source.to_owned(),
        name: name.to_owned(),
        parameters,
        result: result.to_owned(),
    })
}

/// Add an opt-in target public function with compiler-inferred Into conversions.
/// This never modifies the imported source or its existing signatures. Only a
/// successful isolated Cargo test run may publish the resulting new target.
pub(crate) fn apply_verified_api_signatures(
    staged: &Path,
    module_name: &str,
    requested: &[String],
    aliases: &[String],
) -> Result<Vec<String>, VerifiedRustError> {
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    if requested.len() + aliases.len() > MAX_ADAPTERS {
        return Err(VerifiedRustError::Unsafe(
            "combined aliases/signature adapters exceed eight exports".into(),
        ));
    }
    let lib = staged.join("src/lib.rs");
    let metadata = fs::symlink_metadata(&lib).map_err(|_| {
        VerifiedRustError::Unsafe("signature adapters require a library target".into())
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(VerifiedRustError::Unsafe(
            "signature adapter target entrypoint is not a regular file".into(),
        ));
    }
    let original = fs::read_to_string(&lib)?;
    let mut names = BTreeSet::new();
    for alias in aliases {
        let (_, name) = alias
            .split_once('=')
            .ok_or_else(|| VerifiedRustError::Unsafe("invalid previous alias rule".into()))?;
        names.insert(name.to_owned());
    }
    let mut parsed = Vec::new();
    // Validate every rule, and all collisions, before writing to staging.
    for item in requested {
        let adapter = parse_rule(item)?;
        if adapter.name == module_name || !names.insert(adapter.name.clone()) {
            return Err(VerifiedRustError::Unsafe(format!(
                "signature adapter destination collision: {}",
                adapter.name
            )));
        }
        parsed.push(adapter);
    }
    let mut updated = original;
    for item in &parsed {
        let params = item
            .parameters
            .iter()
            .enumerate()
            .map(|(index, ty)| format!("arg{index}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ");
        let arguments = item
            .parameters
            .iter()
            .enumerate()
            .map(|(index, _)| format!("::core::convert::Into::into(arg{index})"))
            .collect::<Vec<_>>()
            .join(", ");
        // Safe and intentionally limited: Rust chooses each source argument
        // type and only accepts an actual Into conversion (never a lossy as).
        // The requested output conversion is also typechecked by Rust.
        updated.push_str(&format!(
            "\n// Software Composer: explicit target signature, compiler-inferred conversions\n\
             pub fn {}({params}) -> {} {{\n\
             let adapted_result: {} = crate::{module_name}::{}({arguments}).into();\n\
             adapted_result\n}}\n",
            item.name, item.result, item.result, item.source
        ));
    }
    fs::write(&lib, updated)?;
    Ok(requested.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bounded_contracts_and_rejects_expressions() {
        let spec = parse_rule("math::double=adapted(u16)->u64").unwrap();
        assert_eq!(spec.source, "math::double");
        assert_eq!(spec.name, "adapted");
        assert_eq!(spec.parameters, vec!["u16"]);
        assert_eq!(spec.result, "u64");
        assert_eq!(
            parse_rule("math::yes=bool_value()->bool")
                .unwrap()
                .parameters
                .len(),
            0
        );
        for invalid in [
            "math::double=bad(u16;panic!())->u64",
            "math::double=bad(Vec<u8>)->u64",
            "math::double=bad(u64)->u64;unsafe{}",
            "math::double=bad(u16)->&str",
            "math::double=bad(u16,u16,u16,u16,u16)->u64",
            "math::double=bad(u16)->u64\npub fn evil(){}",
            "../foo=bad(u16)->u64",
        ] {
            assert!(parse_rule(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn rejects_collisions_without_modifying_original_target() {
        let root = std::env::temp_dir().join(format!("composer-signatures-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("src")).unwrap();
        let lib = root.join("src/lib.rs");
        fs::write(&lib, "pub fn existing() {}\n").unwrap();
        let failed = apply_verified_api_signatures(
            &root,
            "composer_demo",
            &[
                "math::double=adapted(u16)->u64".into(),
                "math::other=adapted(u16)->u64".into(),
            ],
            &[],
        );
        assert!(matches!(failed, Err(VerifiedRustError::Unsafe(_))));
        assert_eq!(fs::read_to_string(&lib).unwrap(), "pub fn existing() {}\n");
        let failed = apply_verified_api_signatures(
            &root,
            "composer_demo",
            &["math::double=adapted(u16)->u64".into()],
            &["math::double=adapted".into()],
        );
        assert!(failed.is_err());
        assert_eq!(fs::read_to_string(&lib).unwrap(), "pub fn existing() {}\n");
        let added = apply_verified_api_signatures(
            &root,
            "composer_demo",
            &["math::double=adapted(u16)->u64".into()],
            &[],
        )
        .unwrap();
        assert_eq!(added.len(), 1);
        let result = fs::read_to_string(&lib).unwrap();
        assert!(result.contains("pub fn adapted(arg0: u16) -> u64"));
        assert!(result.contains("let adapted_result: u64"));
        fs::remove_dir_all(root).unwrap();
    }
}
