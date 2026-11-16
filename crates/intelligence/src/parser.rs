use std::path::Path;

use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser};

use crate::{
    SourceLanguage,
    model::{Symbol, SymbolKind},
};

#[derive(Debug, Clone)]
pub(crate) struct RawImport {
    pub target: String,
    pub kind: String,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct RawReference {
    pub target_name: String,
    pub line: usize,
    pub from_symbol_id: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct ParsedSource {
    pub symbols: Vec<Symbol>,
    pub imports: Vec<RawImport>,
    pub references: Vec<RawReference>,
    pub syntax_error_count: usize,
}

pub(crate) fn parse_source(
    language: SourceLanguage,
    path: &Path,
    source: &str,
) -> Result<ParsedSource, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.grammar())
        .map_err(|error| format!("could not load {:?} grammar: {error}", language))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Tree-sitter cancelled parsing".to_owned())?;

    let mut parsed = ParsedSource {
        syntax_error_count: count_syntax_errors(tree.root_node()),
        ..ParsedSource::default()
    };

    walk(tree.root_node(), source, language, path, None, &mut parsed);

    Ok(parsed)
}

fn walk(
    node: Node<'_>,
    source: &str,
    language: SourceLanguage,
    path: &Path,
    enclosing_symbol_id: Option<String>,
    parsed: &mut ParsedSource,
) {
    let source_bytes = source.as_bytes();
    let symbol = symbol_for_node(
        node,
        source_bytes,
        language,
        path,
        enclosing_symbol_id.as_deref(),
    );
    let current_symbol_id = symbol
        .as_ref()
        .map(|value| value.id.clone())
        .or(enclosing_symbol_id);

    if let Some(symbol) = symbol {
        parsed.symbols.push(symbol);
    }

    if is_import_node(node, language, source_bytes) {
        for target in import_targets(node, language, source_bytes) {
            if !target.trim().is_empty() {
                parsed.imports.push(RawImport {
                    target,
                    kind: node.kind().to_owned(),
                    line: node.start_position().row + 1,
                });
            }
        }
    }

    if is_call_node(node, language)
        && let Some(target_name) = call_target_name(node, source_bytes)
    {
        parsed.references.push(RawReference {
            target_name,
            line: node.start_position().row + 1,
            from_symbol_id: current_symbol_id.clone(),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(
            child,
            source,
            language,
            path,
            current_symbol_id.clone(),
            parsed,
        );
    }
}

fn symbol_for_node(
    node: Node<'_>,
    source: &[u8],
    language: SourceLanguage,
    path: &Path,
    parent_symbol_id: Option<&str>,
) -> Option<Symbol> {
    let kind = symbol_kind(node, language, parent_symbol_id.is_some(), source)?;
    let name_node = if language == SourceLanguage::Go && node.kind() == "type_spec" {
        node.child_by_field_name("name")
    } else if language == SourceLanguage::Python && node.kind() == "assignment" {
        node.child_by_field_name("left")
    } else {
        node.child_by_field_name("name")
    }?;

    let name = source_text(name_node, source)?.trim();
    let name = name
        .split(',')
        .next()
        .unwrap_or(name)
        .trim()
        .trim_matches(['(', ')']);

    if name.is_empty() || name.len() > 160 {
        return None;
    }

    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update([0]);
    hasher.update(node.kind().as_bytes());
    hasher.update([0]);
    hasher.update(name.as_bytes());
    hasher.update(node.start_byte().to_le_bytes());
    let digest = hasher.finalize();
    let id = format!("sym-{}", hex_prefix(&digest, 12));

    Some(Symbol {
        id,
        name: name.to_owned(),
        kind,
        file_path: path.to_path_buf(),
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row.max(node.start_position().row) + 1,
        parent_symbol_id: parent_symbol_id.map(str::to_owned),
    })
}

fn symbol_kind(
    node: Node<'_>,
    language: SourceLanguage,
    nested: bool,
    source: &[u8],
) -> Option<SymbolKind> {
    let kind = node.kind();
    match language {
        SourceLanguage::Rust => match kind {
            "function_item" | "function_signature_item" => Some(if nested {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            }),
            "struct_item" | "union_item" => Some(SymbolKind::Struct),
            "enum_item" => Some(SymbolKind::Enum),
            "trait_item" => Some(SymbolKind::Trait),
            "type_item" => Some(SymbolKind::TypeAlias),
            "mod_item" => Some(SymbolKind::Module),
            "const_item" | "static_item" => Some(SymbolKind::Constant),
            "macro_definition" => Some(SymbolKind::Macro),
            _ => None,
        },
        SourceLanguage::Python => match kind {
            "function_definition" => Some(if nested {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            }),
            "class_definition" => Some(SymbolKind::Class),
            "assignment" if !nested => Some(SymbolKind::Variable),
            _ => None,
        },
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => match kind
        {
            "function_declaration" | "generator_function_declaration" => Some(SymbolKind::Function),
            "class_declaration" => Some(SymbolKind::Class),
            "interface_declaration" => Some(SymbolKind::Interface),
            "type_alias_declaration" => Some(SymbolKind::TypeAlias),
            "enum_declaration" => Some(SymbolKind::Enum),
            "method_definition" => Some(SymbolKind::Method),
            "variable_declarator" if is_top_level_variable(node) => {
                let value = node.child_by_field_name("value");
                if value.is_some_and(|child| {
                    matches!(child.kind(), "arrow_function" | "function_expression")
                }) {
                    Some(SymbolKind::Function)
                } else {
                    Some(variable_kind(node, source))
                }
            }
            _ => None,
        },
        SourceLanguage::Go => match kind {
            "function_declaration" => Some(SymbolKind::Function),
            "method_declaration" => Some(SymbolKind::Method),
            "type_spec" => {
                let value = node.child_by_field_name("type")?;
                match value.kind() {
                    "struct_type" => Some(SymbolKind::Struct),
                    "interface_type" => Some(SymbolKind::Interface),
                    _ => Some(SymbolKind::TypeAlias),
                }
            }
            "const_spec" => Some(SymbolKind::Constant),
            "var_spec" => Some(SymbolKind::Variable),
            _ => None,
        },
    }
}

fn is_top_level_variable(node: Node<'_>) -> bool {
    let Some(declaration) = node.parent() else {
        return false;
    };
    if !matches!(
        declaration.kind(),
        "lexical_declaration" | "variable_declaration"
    ) {
        return false;
    }
    let Some(parent) = declaration.parent() else {
        return false;
    };
    parent.kind() == "program" || parent.kind() == "export_statement"
}

fn variable_kind(node: Node<'_>, source: &[u8]) -> SymbolKind {
    let declaration = node.parent();
    let text = declaration
        .and_then(|value| value.utf8_text(source).ok())
        .unwrap_or_default();
    if text.trim_start().starts_with("const") {
        SymbolKind::Constant
    } else {
        SymbolKind::Variable
    }
}

fn is_import_node(node: Node<'_>, language: SourceLanguage, source: &[u8]) -> bool {
    match language {
        SourceLanguage::Rust => {
            node.kind() == "use_declaration"
                || (node.kind() == "mod_item"
                    && node.child_by_field_name("body").is_none()
                    && source_text(node, source).is_some_and(|text| text.trim_end().ends_with(';')))
        }
        SourceLanguage::Python => {
            matches!(node.kind(), "import_statement" | "import_from_statement")
        }
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            node.kind() == "import_statement"
                || (node.kind() == "export_statement"
                    && node.child_by_field_name("source").is_some())
        }
        SourceLanguage::Go => node.kind() == "import_spec",
    }
}

fn import_targets(node: Node<'_>, language: SourceLanguage, source: &[u8]) -> Vec<String> {
    match language {
        SourceLanguage::Rust => {
            let Some(text) = source_text(node, source) else {
                return Vec::new();
            };
            let raw = text.trim();
            let use_offset = raw.find("use ");
            let mod_offset = raw.find("mod ");
            let (keyword_offset, keyword) = match (use_offset, mod_offset) {
                (Some(offset), _) => (offset, "use "),
                (None, Some(offset)) => (offset, "mod "),
                (None, None) => return Vec::new(),
            };
            let target = raw[keyword_offset + keyword.len()..]
                .trim()
                .split('{')
                .next()
                .unwrap_or_default()
                .trim()
                .trim_end_matches(';')
                .trim()
                .trim_end_matches("::")
                .to_owned();
            if target.is_empty() {
                Vec::new()
            } else {
                vec![target]
            }
        }
        SourceLanguage::Python => {
            let Some(text) = source_text(node, source) else {
                return Vec::new();
            };
            if node.kind() == "import_from_statement" {
                let text = text.trim().strip_prefix("from ").unwrap_or(text.trim());
                let module = text.split(" import ").next().unwrap_or(text).trim();
                vec![module.to_owned()]
            } else {
                let text = text.trim().strip_prefix("import ").unwrap_or(text.trim());
                text.split(',')
                    .map(|part| {
                        part.trim()
                            .split(" as ")
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_owned()
                    })
                    .filter(|part| !part.is_empty())
                    .collect()
            }
        }
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            find_string_literal(node, source)
                .into_iter()
                .map(|value| unquote(&value))
                .collect()
        }
        SourceLanguage::Go => find_string_literal(node, source)
            .into_iter()
            .map(|value| unquote(&value))
            .collect(),
    }
}

fn find_string_literal(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(
        node.kind(),
        "string" | "string_literal" | "interpreted_string_literal" | "raw_string_literal"
    ) {
        return source_text(node, source).map(str::to_owned);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(value) = find_string_literal(child, source) {
            return Some(value);
        }
    }
    None
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        let last = value.as_bytes()[value.len() - 1];
        if (first == b'"' && last == b'"')
            || (first == b'\'' && last == b'\'')
            || (first == b'`' && last == b'`')
        {
            return value[1..value.len() - 1].to_owned();
        }
    }
    value.to_owned()
}

fn is_call_node(node: Node<'_>, language: SourceLanguage) -> bool {
    match language {
        SourceLanguage::Python => node.kind() == "call",
        SourceLanguage::Rust
        | SourceLanguage::JavaScript
        | SourceLanguage::TypeScript
        | SourceLanguage::Tsx
        | SourceLanguage::Go => node.kind() == "call_expression",
    }
}

fn call_target_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let function = node.child_by_field_name("function")?;
    let text = source_text(function, source)?.trim();
    let normalized = text
        .trim_end_matches('!')
        .split(['.', ':'])
        .rfind(|part| !part.is_empty())
        .unwrap_or(text)
        .trim();

    if normalized.is_empty()
        || normalized.len() > 160
        || !normalized
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return None;
    }

    Some(normalized.to_owned())
}

fn source_text<'a>(node: Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    node.utf8_text(source).ok()
}

fn count_syntax_errors(node: Node<'_>) -> usize {
    let mut count = usize::from(node.is_error() || node.is_missing());
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_syntax_errors(child);
    }
    count
}

fn hex_prefix(bytes: &[u8], length: usize) -> String {
    bytes
        .iter()
        .take(length)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(language: SourceLanguage, file: &str, source: &str) -> ParsedSource {
        let result = parse_source(language, Path::new(file), source).unwrap();
        assert_eq!(result.syntax_error_count, 0, "source should parse cleanly");
        result
    }

    #[test]
    fn parses_rust_symbols_imports_and_calls() {
        let result = parse(
            SourceLanguage::Rust,
            "src/auth.rs",
            "use crate::token::verify_token;\npub struct AuthState;\npub fn authenticate_user() { verify_token(); }\n",
        );

        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.name == "AuthState")
        );
        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.name == "authenticate_user")
        );
        assert!(
            result
                .imports
                .iter()
                .any(|item| item.target.contains("crate::token"))
        );
        assert!(
            result
                .references
                .iter()
                .any(|item| item.target_name == "verify_token")
        );
    }

    #[test]
    fn parses_python_classes_functions_and_imports() {
        let result = parse(
            SourceLanguage::Python,
            "auth.py",
            "from .tokens import verify_token\nclass AuthService:\n    def authenticate_user(self):\n        verify_token()\n",
        );

        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.name == "AuthService")
        );
        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.name == "authenticate_user")
        );
        assert!(result.imports.iter().any(|item| item.target == ".tokens"));
        assert!(
            result
                .references
                .iter()
                .any(|item| item.target_name == "verify_token")
        );
    }

    #[test]
    fn parses_javascript_exports_imports_and_calls() {
        let result = parse(
            SourceLanguage::JavaScript,
            "src/auth.js",
            "import { verifyToken } from './token.js';\nexport function authenticateUser() { verifyToken(); }\n",
        );

        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.name == "authenticateUser")
        );
        assert!(
            result
                .imports
                .iter()
                .any(|item| item.target == "./token.js")
        );
        assert!(
            result
                .references
                .iter()
                .any(|item| item.target_name == "verifyToken")
        );
    }

    #[test]
    fn parses_typescript_interfaces_and_functions() {
        let result = parse(
            SourceLanguage::TypeScript,
            "src/auth.ts",
            "export interface User { id: string }\nexport function signIn(user: User): User { return user; }\n",
        );

        assert!(result.symbols.iter().any(|symbol| symbol.name == "User"));
        assert!(result.symbols.iter().any(|symbol| symbol.name == "signIn"));
    }

    #[test]
    fn parses_tsx_components() {
        let result = parse(
            SourceLanguage::Tsx,
            "src/UserCard.tsx",
            "type Props = { name: string };\nexport function UserCard(props: Props) { return <div>{props.name}</div>; }\n",
        );

        assert!(result.symbols.iter().any(|symbol| symbol.name == "Props"));
        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.name == "UserCard")
        );
    }

    #[test]
    fn parses_go_functions_and_imports() {
        let result = parse(
            SourceLanguage::Go,
            "auth/auth.go",
            "package auth\nimport \"context\"\nfunc AuthenticateUser(ctx context.Context) error { return nil }\n",
        );

        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.name == "AuthenticateUser")
        );
        assert!(result.imports.iter().any(|item| item.target == "context"));
    }
}
