//! Read-only Rust API contract discovery. Produces *suggestions*, never
//! executable code, and does not modify, compile, or trust third-party sources.
//! Only explicitly unfinished public library entrypoints are considered target
//! requirements. Actual execution requires reviewed CLI mappings plus Docker
//! compiler, contract-test, and publication gates.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};
use thiserror::Error;
use tree_sitter::{Node, Parser, Tree};

use crate::verify::{VerificationError, verify_capsule};

const MAX_TARGET_BYTES: u64 = 1024 * 1024;
const MAX_CANDIDATES: usize = 5;
const MAX_INPUT_FUNCTIONS: usize = 256;
const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum RustApiDiscoveryError {
    #[error("capsule validation failed: {0}")]
    Capsule(#[from] VerificationError),
    #[error("file access failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported contract: {0}")]
    Unsupported(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustApiSuggestion {
    pub source_file: PathBuf,
    pub source_function: String,
    pub source_parameters: Vec<String>,
    pub source_return_type: String,
    pub suggested_cli_rule: String,
    /// Deterministic ranking weight, NOT a probability of correctness.
    pub ranking_score: u32,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustTargetRequirement {
    pub target_function: String,
    pub parameter_types: Vec<String>,
    pub return_type: String,
    pub status: String,
    pub best_suggestion: Option<RustApiSuggestion>,
    pub candidates: Vec<RustApiSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustApiDiscoveryReport {
    pub schema_version: u32,
    pub capsule_id: String,
    pub target_root: PathBuf,
    pub source_functions_inspected: usize,
    pub target_stubs_inspected: usize,
    pub requirements: Vec<RustTargetRequirement>,
    pub warning: String,
}

#[derive(Debug, Clone)]
struct ApiFunction {
    name: String,
    parameters: Vec<String>,
    result: String,
    file: PathBuf,
    source_path: String,
    body_is_unimplemented: bool,
}

pub fn discover_rust_api_contracts(
    capsule_root: &Path,
    target_root: &Path,
) -> Result<RustApiDiscoveryReport, RustApiDiscoveryError> {
    let verified = verify_capsule(capsule_root)?;
    let target_root = target_root.canonicalize()?;
    let src_directory = target_root.join("src");
    let directory_meta = fs::symlink_metadata(&src_directory)?;
    if !directory_meta.is_dir() || directory_meta.file_type().is_symlink() {
        return Err(RustApiDiscoveryError::Unsupported(
            "target src/ must be a regular, non-symlink directory".into(),
        ));
    }
    let lib = src_directory.join("lib.rs");
    let meta = fs::symlink_metadata(&lib)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_TARGET_BYTES {
        return Err(RustApiDiscoveryError::Unsupported(
            "target requires a regular, non-symlink src/lib.rs of at most 1 MiB".into(),
        ));
    }
    let lib_text = fs::read_to_string(&lib)?;
    let mut requirements = scan_public_functions(&lib_text, Path::new("src/lib.rs"), "")
        .into_iter()
        .filter(|func| func.body_is_unimplemented)
        .collect::<Vec<_>>();
    if requirements.len() > MAX_INPUT_FUNCTIONS {
        return Err(RustApiDiscoveryError::Unsupported(
            "target has more than 256 unfinished public Rust API requirements".into(),
        ));
    }

    let sources = verified
        .manifest
        .files
        .iter()
        .filter(|entry| entry.file_kind == "source")
        .collect::<Vec<_>>();
    let multifile = sources.len() > 1;
    let mut source_functions = Vec::new();
    let mut source_bytes = 0;
    for entry in sources {
        let Some(path) = entry.source_path.as_deref() else {
            continue;
        };
        if path.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let Some(module) = module_path(path, multifile) else {
            continue;
        };
        let original = verified.root.join(&entry.capsule_path);
        let meta = fs::symlink_metadata(&original)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(RustApiDiscoveryError::Unsupported(format!(
                "non-regular source file {}",
                original.display()
            )));
        }
        source_bytes += meta.len();
        if source_bytes > MAX_SOURCE_BYTES {
            return Err(RustApiDiscoveryError::Unsupported(
                "selected Rust API source exceeds 4 MiB".into(),
            ));
        }
        let source = fs::read_to_string(original)?;
        source_functions.extend(scan_public_functions(&source, path, &module));
        if source_functions.len() > MAX_INPUT_FUNCTIONS {
            return Err(RustApiDiscoveryError::Unsupported(
                "selected Rust API contains more than 256 public functions".into(),
            ));
        }
    }
    source_functions.sort_by(|a, b| a.file.cmp(&b.file).then(a.source_path.cmp(&b.source_path)));

    let mut results = Vec::new();
    for target in requirements.drain(..) {
        let mut candidates = Vec::new();
        for source in &source_functions {
            let Some(ranking) = rank_candidate(&target, source) else {
                continue;
            };
            let rule = format!(
                "{}={}({})->{}",
                source.source_path,
                target.name,
                target.parameters.join(","),
                target.result,
            );
            candidates.push(RustApiSuggestion {
                source_file: source.file.clone(),
                source_function: source.name.clone(),
                source_parameters: source.parameters.clone(),
                source_return_type: source.result.clone(),
                suggested_cli_rule: rule,
                ranking_score: ranking,
                rationale: "Name-token overlap and only fixed-width, infallible primitive widening/identity; does not prove semantic equivalence".into(),
            });
        }
        candidates.sort_by(|a, b| {
            b.ranking_score
                .cmp(&a.ranking_score)
                .then(a.source_file.cmp(&b.source_file))
                .then(a.source_function.cmp(&b.source_function))
        });
        candidates.truncate(MAX_CANDIDATES);
        let ambiguous =
            candidates.len() > 1 && candidates[0].ranking_score == candidates[1].ranking_score;
        let best = if !ambiguous && candidates.first().is_some_and(|c| c.ranking_score >= 70) {
            candidates.first().cloned()
        } else {
            None
        };
        let status = if best.is_some() {
            "candidate_requires_independent_review_and_compiler_verification"
        } else if ambiguous {
            "ambiguous_candidates_no_automatic_selection"
        } else {
            "no_supported_unambiguous_candidate"
        };
        results.push(RustTargetRequirement {
            target_function: target.name,
            parameter_types: target.parameters,
            return_type: target.result,
            status: status.into(),
            best_suggestion: best,
            candidates,
        });
    }
    Ok(RustApiDiscoveryReport {
        schema_version: 1,
        capsule_id: verified.manifest.capsule_id,
        target_root,
        source_functions_inspected: source_functions.len(),
        target_stubs_inspected: results.len(),
        requirements: results,
        warning: "Read-only heuristic recommendations; no code generated or verified. Names alone never establish behavior. Review each contract and require independent Cargo compilation and behavioral tests before publication.".into(),
    })
}

/// Infer only a top-level, public function with borrowed &str inputs and a
/// supported owned scalar/boolean/String result using the existing Tree-sitter
/// Rust parser. This is type-shape inference, never semantic equivalence;
/// the caller explicitly selects the source symbol and target alias, and
/// must supply independent behavioral tests before publishing.
/// Methods, modules, generics, async, unsafe, attrs that alter function shape,
/// and unsupported argument or return types fail closed.
/// Typed metadata for one uniquely resolved, top-level public Rust API.
/// Only stable primitive/owned values and immutable borrowed strings are
/// supported. Neither function names nor signatures establish meaning.
#[derive(Debug, Clone)]
pub(crate) struct InferredGitSignature {
    pub(crate) target_signature: String,
    pub(crate) source_parameters: Vec<String>,
}

/// Reuse Composer's existing Tree-sitter parser to extract a *type-only* call
/// bridge from a reviewed, SHA-pinned upstream function. There is no algorithm
/// generation and no trust in matching names as semantic proof: independent
/// target-owned assertions remain mandatory for publication.
pub(crate) fn infer_typed_git_signature(
    pinned_source: &str,
    upstream_symbol: &str,
    target_alias: &str,
) -> Option<InferredGitSignature> {
    if !identifier(upstream_symbol) || !identifier(target_alias) {
        return None;
    }
    let mut matches = scan_public_functions(pinned_source, Path::new("src/lib.rs"), "")
        .into_iter()
        .filter(|candidate| {
            candidate.name == upstream_symbol
                && candidate.parameters.len() <= 4
                && candidate
                    .parameters
                    .iter()
                    .all(|ty| ty == "&str" || (allowed_type(ty) && ty != "()" && ty != "&str"))
                && allowed_type(&candidate.result)
                && candidate.result != "&str"
        });
    let matched = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let parameters = matched
        .parameters
        .iter()
        .map(|param| {
            if param == "&str" {
                "String"
            } else {
                param.as_str()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    Some(InferredGitSignature {
        target_signature: format!("{target_alias}({parameters})->{}", matched.result),
        source_parameters: matched.parameters,
    })
}

pub(crate) fn infer_owned_string_target_signature(
    pinned_source: &str,
    upstream_symbol: &str,
    target_alias: &str,
) -> Option<String> {
    if !identifier(upstream_symbol) || !identifier(target_alias) {
        return None;
    }
    let mut matches = scan_public_functions(pinned_source, Path::new("src/lib.rs"), "")
        .into_iter()
        .filter(|candidate| {
            candidate.name == upstream_symbol
                && matches!(
                    candidate.result.as_str(),
                    "String"
                        | "bool"
                        | "f32"
                        | "f64"
                        | "u8"
                        | "u16"
                        | "u32"
                        | "u64"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                )
                && !candidate.parameters.is_empty()
                && candidate.parameters.iter().all(|ty| ty == "&str")
        });
    let candidate = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let inputs = vec!["String"; candidate.parameters.len()].join(",");
    Some(format!("{target_alias}({inputs})->{}", candidate.result))
}

fn rust_tree(source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(source, None)?;
    (!tree.root_node().has_error()).then_some(tree)
}

/// Visit root-level functions only. Methods, nested modules, impl blocks,
/// generics, lifetimes, unsafe and async functions cannot be safely mapped.
fn scan_public_functions(source: &str, file: &Path, module: &str) -> Vec<ApiFunction> {
    let Some(tree) = rust_tree(source) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .filter_map(|node| parse_function(node, source, file, module))
        .collect()
}

fn parse_function(node: Node<'_>, source: &str, file: &Path, module: &str) -> Option<ApiFunction> {
    if node.kind() != "function_item" {
        return None;
    }
    let name = node.child_by_field_name("name")?;
    let args = node.child_by_field_name("parameters")?;
    let body = node.child_by_field_name("body")?;
    let prefix = source.get(node.start_byte()..name.start_byte())?.trim();
    // Conservative: do not infer across visibility, const/async/unsafe, ABI,
    // generics or attributes that alter calling conventions.
    if prefix != "pub fn"
        || !source
            .get(name.end_byte()..args.start_byte())?
            .trim()
            .is_empty()
    {
        // Reject type parameters and any syntax between name and arguments;
        // no generic/source specialization may be inferred from a simple call.
        return None;
    }
    let name = name.utf8_text(source.as_bytes()).ok()?;
    if !identifier(name) {
        return None;
    }
    let body_text = body.utf8_text(source.as_bytes()).ok()?;
    let compact = body_text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    let body_is_unimplemented = matches!(
        compact.as_str(),
        "{todo!()}" | "{todo!();}" | "{unimplemented!()}" | "{unimplemented!();}"
    );
    let param_text = source.get(args.start_byte() + 1..args.end_byte().checked_sub(1)?)?;
    let mut params = Vec::new();
    if !param_text.trim().is_empty() {
        for param in param_text.split(',') {
            let (variable, ty) = param.trim().split_once(':')?;
            // Mutation of a local binding does not change the public type.
            // Accept only a simple optional `mut` prefix, never destructuring.
            let binding = variable
                .trim()
                .strip_prefix("mut ")
                .unwrap_or(variable.trim());
            if !identifier(binding) || !allowed_type(ty.trim()) {
                return None;
            }
            params.push(ty.trim().to_owned());
        }
    }
    if params.len() > 4 {
        return None;
    }
    let after = source.get(args.end_byte()..body.start_byte())?.trim();
    let result = after.strip_prefix("->").map(str::trim).unwrap_or("()");
    if !allowed_type(result) || (!after.is_empty() && !after.starts_with("->")) {
        return None;
    }
    Some(ApiFunction {
        name: name.to_owned(),
        parameters: params,
        result: result.to_owned(),
        file: file.to_path_buf(),
        source_path: if module.is_empty() {
            name.to_owned()
        } else {
            format!("{module}::{name}")
        },
        body_is_unimplemented,
    })
}

fn identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        && value != "_"
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn allowed_type(value: &str) -> bool {
    matches!(
        value,
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
            | "bool"
            | "char"
            | "String"
            | "&str"
            | "()"
    )
}
fn safe_into(input: &str, destination: &str) -> bool {
    if input == destination {
        return true;
    }
    if input == "&str" && destination == "String" {
        return true;
    }
    let rank = |type_name: &str| -> Option<(u8, usize)> {
        for (prefix, signedness) in [("u", 0), ("i", 1)] {
            let Some(width) = type_name.strip_prefix(prefix) else {
                continue;
            };
            let bits = width.parse::<usize>().ok()?;
            if matches!(bits, 8 | 16 | 32 | 64 | 128) {
                return Some((signedness, bits));
            }
        }
        None
    };
    matches!((rank(input), rank(destination)),
        (Some((sign_a, bits_a)), Some((sign_b, bits_b)))
        if sign_a == sign_b && bits_a < bits_b)
}
fn name_score(a: &str, b: &str) -> u32 {
    if a == b {
        return 100;
    }
    let tokenize = |name: &str| {
        name.to_ascii_lowercase()
            .split('_')
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    };
    let a = tokenize(a);
    let b = tokenize(b);
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let common = a.intersection(&b).count();
    if common == 0 {
        return 0;
    }
    let longer = a.len().max(b.len());
    if common == longer {
        90
    } else if common >= 2 {
        80
    } else if longer <= 2 {
        70
    } else {
        45
    }
}
fn rank_candidate(target: &ApiFunction, source: &ApiFunction) -> Option<u32> {
    if target.parameters.len() != source.parameters.len() {
        return None;
    }
    for (target_input, source_input) in target.parameters.iter().zip(&source.parameters) {
        if !safe_into(target_input, source_input) {
            return None;
        }
    }
    if !safe_into(&source.result, &target.result) {
        return None;
    }
    let score = name_score(&source.name, &target.name);
    (score >= 45).then_some(score)
}

fn module_path(path: &Path, multifile: bool) -> Option<String> {
    let mut components = path.components();
    let Component::Normal(root) = components.next()? else {
        return None;
    };
    if root != "src" {
        return None;
    }
    let mut names = Vec::new();
    for part in components {
        let Component::Normal(value) = part else {
            return None;
        };
        let value = value.to_str()?;
        if value.starts_with('.') {
            return None;
        }
        names.push(value.to_owned());
    }
    let leaf = names.pop()?;
    if !leaf.ends_with(".rs") {
        return None;
    }
    let stem = leaf.strip_suffix(".rs")?;
    if !multifile {
        return Some(String::new());
    }
    if !matches!(stem, "lib" | "main" | "mod") {
        names.push(stem.into());
    }
    if !names.iter().all(|x| identifier(x)) {
        return None;
    }
    Some(names.join("::"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mock(name: &str, p: &[&str], ret: &str) -> ApiFunction {
        ApiFunction {
            name: name.into(),
            parameters: p.iter().map(|x| x.to_string()).collect(),
            result: ret.into(),
            file: "src/math.rs".into(),
            source_path: name.into(),
            body_is_unimplemented: false,
        }
    }

    #[test]
    fn infers_only_exact_upstream_borrowed_string_signatures() {
        let source = r#"
#[inline(always)]
#[cfg(feature = "alloc")]
pub fn deunicode(s: &str) -> String { s.to_owned() }
pub fn combine(a: &str, b: &str) -> String { format!("{a}{b}") }
pub fn similarity(a: &str, b: &str) -> f64 { if a == b { 1.0 } else { 0.0 } }
pub fn equal(a: &str, b: &str) -> bool { a == b }
pub fn byte_count(s: &str) -> u64 { s.len() as u64 }
pub fn wrong(arg: u32) -> String { arg.to_string() }
pub fn generic<T>(s: &str) -> String { s.to_owned() }
pub fn reference(s: &str) -> &str { s }
pub async fn delayed(s: &str) -> String { s.to_owned() }
struct Internal;
impl Internal { pub fn method(s: &str) -> String { s.to_owned() } }
"#;
        assert_eq!(
            infer_owned_string_target_signature(source, "deunicode", "transliterate"),
            Some("transliterate(String)->String".into())
        );
        assert_eq!(
            infer_owned_string_target_signature(source, "combine", "concat"),
            Some("concat(String,String)->String".into())
        );
        assert_eq!(
            infer_owned_string_target_signature(source, "similarity", "ported_similarity"),
            Some("ported_similarity(String,String)->f64".into())
        );
        assert_eq!(
            infer_owned_string_target_signature(source, "equal", "is_equal"),
            Some("is_equal(String,String)->bool".into())
        );
        assert_eq!(
            infer_owned_string_target_signature(source, "byte_count", "bytes"),
            Some("bytes(String)->u64".into())
        );
        for unsupported in [
            "wrong",
            "generic",
            "reference",
            "delayed",
            "method",
            "missing",
            "deunicode;evil",
        ] {
            assert!(
                infer_owned_string_target_signature(source, unsupported, "target").is_none(),
                "{unsupported}"
            );
        }
        assert!(
            infer_owned_string_target_signature(source, "deunicode", "unsafe::target").is_none()
        );
    }

    #[test]
    fn identifies_mixed_primitive_and_borrowed_string_calls_without_semantic_guessing() {
        let src = r#"
pub fn distance(a: &str, b: &str) -> usize { a.len() + b.len() }
pub fn limit(input: &str, max: usize, strict: bool) -> bool { input.len() < max && strict }
pub fn shift(n: u32, delta: i32) -> i64 { n as i64 + delta as i64 }
pub fn consume(value: String) -> String { value }
pub fn nullary() -> bool { true }
pub fn generic<T>(value: T) -> bool { true }
pub async fn async_fn(value: usize) -> bool { true }
struct Item;
impl Item { pub fn method(value: usize) -> usize { value } }
pub fn borrowed(value: &str) -> &str { value }
"#;
        let cases = [
            ("distance", "text_distance(String,String)->usize"),
            ("limit", "capped(String,usize,bool)->bool"),
            ("shift", "shifted(u32,i32)->i64"),
            ("consume", "consumed(String)->String"),
            ("nullary", "no_args()->bool"),
        ];
        for (source, expected) in cases {
            let target = match source {
                "distance" => "text_distance",
                "limit" => "capped",
                "shift" => "shifted",
                "consume" => "consumed",
                _ => "no_args",
            };
            assert_eq!(
                infer_typed_git_signature(src, source, target)
                    .unwrap()
                    .target_signature,
                expected
            );
        }
        assert_eq!(
            infer_typed_git_signature(src, "limit", "capped")
                .unwrap()
                .source_parameters,
            vec!["&str", "usize", "bool"]
        );
        for name in ["generic", "async_fn", "method", "borrowed", "missing"] {
            assert!(infer_typed_git_signature(src, name, "target").is_none());
        }
        assert!(infer_typed_git_signature(src, "shift", "bad::alias").is_none());
    }

    #[test]
    fn extracts_only_public_top_level_and_explicit_unimplemented_stubs() {
        let code = r#"pub fn totient(value: u32) -> u64 { todo!() }
pub(crate) fn secret(x: u32) -> u32 { todo!() }
pub fn implemented(x: u32) -> u64 { x.into() }
pub fn more(x: u64) -> u64 { unimplemented!(); }
struct Item {}
impl Item { pub fn method(&self) -> u32 { 2 } }
pub async fn delayed(x: u32) -> u32 { todo!() }
"#;
        let functions = scan_public_functions(code, Path::new("src/lib.rs"), "");
        assert_eq!(functions.len(), 3);
        let mutable = scan_public_functions(
            "pub fn fast_gcd(mut a: i64, mut b: i64) -> i64 { a + b }",
            Path::new("src/math.rs"),
            "",
        );
        assert_eq!(mutable.len(), 1);
        assert_eq!(mutable[0].parameters, vec!["i64", "i64"]);
        assert!(functions[0].body_is_unimplemented);
        assert!(!functions[1].body_is_unimplemented);
        assert!(functions[2].body_is_unimplemented);
    }
    #[test]
    fn only_suggests_infallible_widening_with_relevant_names() {
        assert_eq!(
            rank_candidate(
                &mock("totient", &["u32"], "u128"),
                &mock("totient", &["u64"], "u64")
            ),
            Some(100)
        );
        assert_eq!(
            rank_candidate(
                &mock("totient", &["u64"], "u32"),
                &mock("totient", &["u32"], "u64")
            ),
            None
        );
        assert_eq!(
            rank_candidate(
                &mock("distance", &["u32"], "u128"),
                &mock("totient", &["u64"], "u64")
            ),
            None
        );
        assert_eq!(name_score("euler_totient", "adapted_totient"), 70);
    }
    #[test]
    fn module_rules_follow_verified_flat_single_file_layout() {
        assert_eq!(
            module_path(Path::new("src/number_theory/euler_totient.rs"), false),
            Some("".into())
        );
        assert_eq!(
            module_path(Path::new("src/number_theory/euler_totient.rs"), true),
            Some("number_theory::euler_totient".into())
        );
        assert_eq!(module_path(Path::new("src/lib.rs"), true), Some("".into()));
        assert_eq!(module_path(Path::new("../src/lib.rs"), true), None);
    }
}
