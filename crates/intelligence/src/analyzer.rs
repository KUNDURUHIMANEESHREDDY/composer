use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

use thiserror::Error;

use crate::{
    SourceLanguage,
    model::{
        AnalysisSummary, AnalyzedFile, GRAPH_SCHEMA_VERSION, ImportDependency, ImportResolution,
        RepositoryGraph, Symbol, SymbolReference, SymbolResolution,
    },
    parser::{RawReference, parse_source},
};

const MAX_SOURCE_FILES: usize = 50_000;
const MAX_SOURCE_FILE_BYTES: u64 = 1_000_000;
const MAX_TRAVERSAL_DEPTH: usize = 12;

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error("repository root does not exist: {0}")]
    MissingRepository(PathBuf),
    #[error("repository root is not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("failed to inspect repository: {0}")]
    Filesystem(#[from] std::io::Error),
    #[error("repository inventory exceeded the {MAX_SOURCE_FILES} source-file limit")]
    FileLimitExceeded,
}

/// Analyze source files without executing repository-controlled code.
///
/// Symlinks and common generated/dependency directories are skipped. Files are
/// parsed only when they are UTF-8 and within the per-file size limit.
pub fn analyze_repository(repository_root: &Path) -> Result<RepositoryGraph, AnalysisError> {
    if !repository_root.exists() {
        return Err(AnalysisError::MissingRepository(
            repository_root.to_path_buf(),
        ));
    }
    if !repository_root.is_dir() {
        return Err(AnalysisError::NotADirectory(repository_root.to_path_buf()));
    }

    let root = repository_root.canonicalize()?;
    let mut source_paths = Vec::new();
    let mut summary = AnalysisSummary::default();

    walk_source_files(&root, &root, 0, &mut source_paths, &mut summary)?;
    source_paths.sort();

    let mut files = Vec::new();
    let mut symbols = Vec::new();
    let mut raw_imports = Vec::new();
    let mut raw_references = Vec::new();
    let mut languages = BTreeSet::new();

    for relative_path in source_paths {
        let absolute_path = root.join(&relative_path);
        let metadata = fs::metadata(&absolute_path)?;
        if metadata.len() > MAX_SOURCE_FILE_BYTES {
            summary.files_skipped += 1;
            continue;
        }

        let source = match fs::read_to_string(&absolute_path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                summary.files_skipped += 1;
                continue;
            }
            Err(error) => return Err(error.into()),
        };

        let Some(language) = SourceLanguage::from_path(&relative_path) else {
            continue;
        };

        let parsed = match parse_source(language, &relative_path, &source) {
            Ok(parsed) => parsed,
            Err(_) => {
                summary.files_skipped += 1;
                continue;
            }
        };

        let line_count = source.lines().count().max(1);
        if parsed.syntax_error_count > 0 {
            summary.syntax_files_with_errors += 1;
        }

        files.push(AnalyzedFile {
            path: relative_path.clone(),
            language,
            size_bytes: metadata.len(),
            line_count,
            has_syntax_errors: parsed.syntax_error_count > 0,
            syntax_error_count: parsed.syntax_error_count,
        });

        symbols.extend(parsed.symbols);
        raw_imports.push((relative_path.clone(), language, parsed.imports));
        raw_references.push((relative_path, parsed.references));
        languages.insert(language);
    }

    let source_file_set: HashSet<PathBuf> = files.iter().map(|file| file.path.clone()).collect();
    let go_modules = discover_go_modules(&root, &source_file_set);

    let mut imports = Vec::new();
    for (source_file, language, file_imports) in raw_imports {
        for import in file_imports {
            let (resolution, resolved_files) = resolve_import(
                &root,
                &source_file,
                language,
                &import.target,
                &source_file_set,
                &go_modules,
            );
            imports.push(ImportDependency {
                source_file: source_file.clone(),
                target: import.target,
                kind: import.kind,
                line: import.line,
                resolution,
                resolved_file: resolved_files.first().cloned(),
                resolved_files,
            });
        }
    }

    let mut symbols_by_name: HashMap<&str, Vec<&Symbol>> = HashMap::new();
    for symbol in &symbols {
        symbols_by_name
            .entry(symbol.name.as_str())
            .or_default()
            .push(symbol);
    }

    let symbols_by_id: HashMap<&str, &Symbol> = symbols
        .iter()
        .map(|symbol| (symbol.id.as_str(), symbol))
        .collect();

    let mut references = Vec::new();
    for (source_file, file_references) in raw_references {
        for reference in file_references {
            let (resolution, resolved_symbol_id) = resolve_symbol_reference(
                &source_file,
                &reference,
                &symbols_by_name,
                &symbols_by_id,
            );

            references.push(SymbolReference {
                source_file: source_file.clone(),
                from_symbol_id: reference.from_symbol_id,
                target_name: reference.target_name,
                line: reference.line,
                resolved_symbol_id,
                resolution,
            });
        }
    }

    summary.files_analyzed = files.len();
    summary.languages = languages.into_iter().collect();
    summary.symbols_found = symbols.len();
    summary.imports_found = imports.len();
    summary.local_imports_resolved = imports
        .iter()
        .filter(|dependency| dependency.resolution == ImportResolution::Local)
        .count();
    summary.unresolved_imports = imports
        .iter()
        .filter(|dependency| dependency.resolution == ImportResolution::UnresolvedOrExternal)
        .count();
    summary.ambiguous_imports = imports
        .iter()
        .filter(|dependency| dependency.resolution == ImportResolution::Ambiguous)
        .count();
    summary.symbol_references_found = references.len();
    summary.symbol_references_resolved = references
        .iter()
        .filter(|reference| {
            matches!(
                reference.resolution,
                SymbolResolution::ScopedNameMatch
                    | SymbolResolution::SameFileNameMatch
                    | SymbolResolution::UniqueNameMatch
            )
        })
        .count();

    Ok(RepositoryGraph {
        schema_version: GRAPH_SCHEMA_VERSION,
        repository_root: root,
        files,
        symbols,
        imports,
        references,
        summary,
    })
}

fn walk_source_files(
    root: &Path,
    current: &Path,
    depth: usize,
    output: &mut Vec<PathBuf>,
    summary: &mut AnalysisSummary,
) -> Result<(), AnalysisError> {
    if depth > MAX_TRAVERSAL_DEPTH {
        return Ok(());
    }

    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if file_type.is_symlink() || name == ".git" || should_skip_directory(&name) {
            continue;
        }

        let path = entry.path();
        if file_type.is_dir() {
            walk_source_files(root, &path, depth + 1, output, summary)?;
        } else if file_type.is_file() && SourceLanguage::from_path(&path).is_some() {
            if output.len() >= MAX_SOURCE_FILES {
                summary.inventory_truncated = true;
                return Err(AnalysisError::FileLimitExceeded);
            }

            let relative = path
                .strip_prefix(root)
                .expect("visited paths are descendants of the repository root");
            if is_safe_relative_path(relative) {
                output.push(relative.to_path_buf());
            }
        }
    }

    Ok(())
}

fn should_skip_directory(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".composer"
            | ".next"
            | ".turbo"
            | ".venv"
            | "__pycache__"
            | "build"
            | "coverage"
            | "dist"
            | "node_modules"
            | "out"
            | "target"
            | "vendor"
    ) || (name.starts_with('.') && name != ".")
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn normalize_relative_candidate(path: PathBuf) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn resolve_symbol_reference(
    source_file: &Path,
    reference: &RawReference,
    symbols_by_name: &HashMap<&str, Vec<&Symbol>>,
    symbols_by_id: &HashMap<&str, &Symbol>,
) -> (SymbolResolution, Option<String>) {
    let candidates = symbols_by_name
        .get(reference.target_name.as_str())
        .map(Vec::as_slice)
        .unwrap_or_default();

    if let Some(caller) = reference
        .from_symbol_id
        .as_deref()
        .and_then(|symbol_id| symbols_by_id.get(symbol_id))
    {
        let scoped = candidates
            .iter()
            .filter(|candidate| {
                if let Some(parent_id) = caller.parent_symbol_id.as_deref() {
                    candidate.parent_symbol_id.as_deref() == Some(parent_id)
                } else {
                    candidate.parent_symbol_id.as_deref() == Some(caller.id.as_str())
                }
            })
            .copied()
            .collect::<Vec<_>>();

        match scoped.as_slice() {
            [symbol] => {
                return (SymbolResolution::ScopedNameMatch, Some(symbol.id.clone()));
            }
            [] => {}
            _ => return (SymbolResolution::AmbiguousNameMatch, None),
        }
    }

    let same_file = candidates
        .iter()
        .filter(|candidate| candidate.file_path == source_file)
        .copied()
        .collect::<Vec<_>>();

    match same_file.as_slice() {
        [symbol] => (SymbolResolution::SameFileNameMatch, Some(symbol.id.clone())),
        [] => match candidates {
            [symbol] => (SymbolResolution::UniqueNameMatch, Some(symbol.id.clone())),
            [] => (SymbolResolution::Unresolved, None),
            _ => (SymbolResolution::AmbiguousNameMatch, None),
        },
        _ => (SymbolResolution::AmbiguousNameMatch, None),
    }
}

fn resolve_import(
    root: &Path,
    source_file: &Path,
    language: SourceLanguage,
    target: &str,
    source_files: &HashSet<PathBuf>,
    go_modules: &[GoModule],
) -> (ImportResolution, Vec<PathBuf>) {
    let candidates = import_candidates(
        root,
        source_file,
        language,
        target,
        go_modules,
        source_files,
    );
    let matches: Vec<PathBuf> = candidates
        .into_iter()
        .filter_map(normalize_relative_candidate)
        .filter(|candidate| source_files.contains(candidate))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    match (language, matches.is_empty(), matches.len()) {
        (_, true, _) => (ImportResolution::UnresolvedOrExternal, Vec::new()),
        (SourceLanguage::Go, false, _) => (ImportResolution::Local, matches),
        (_, false, 1) => (ImportResolution::Local, matches),
        _ => (ImportResolution::Ambiguous, Vec::new()),
    }
}

fn import_candidates(
    root: &Path,
    source_file: &Path,
    language: SourceLanguage,
    target: &str,
    go_modules: &[GoModule],
    source_files: &HashSet<PathBuf>,
) -> Vec<PathBuf> {
    match language {
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            javascript_candidates(source_file, target)
        }
        SourceLanguage::Python => python_candidates(root, source_file, target),
        SourceLanguage::Rust => rust_candidates(root, source_file, target),
        SourceLanguage::Go => go_candidates(target, go_modules, source_files),
    }
}

fn javascript_candidates(source_file: &Path, target: &str) -> Vec<PathBuf> {
    if !target.starts_with('.') {
        return Vec::new();
    }

    let parent = source_file.parent().unwrap_or_else(|| Path::new(""));
    let base = parent.join(target);
    let mut candidates = vec![base.clone()];

    if base.extension().is_none() {
        for extension in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
            candidates.push(base.with_extension(extension));
            candidates.push(base.join(format!("index.{extension}")));
        }
    }

    candidates
}

fn python_candidates(root: &Path, source_file: &Path, target: &str) -> Vec<PathBuf> {
    let module = target.trim();
    if module.is_empty() {
        return Vec::new();
    }

    let leading_dots = module
        .chars()
        .take_while(|character| *character == '.')
        .count();
    let module_name = module.trim_start_matches('.');
    if module_name.is_empty() {
        // "from . import name" does not identify the imported sibling module
        // in this graph model; keep it unresolved rather than guessing.
        return Vec::new();
    }

    let relative_module = PathBuf::from(module_name.replace('.', "/"));
    let mut bases = Vec::new();

    if leading_dots > 0 {
        let mut base = source_file.parent().unwrap_or_else(|| Path::new(""));
        for _ in 1..leading_dots {
            base = base.parent().unwrap_or_else(|| Path::new(""));
        }
        bases.push(base.to_path_buf());
    } else {
        bases.push(PathBuf::new());
        if let Some(parent) = source_file.parent() {
            bases.push(parent.to_path_buf());
        }
    }

    let mut candidates = Vec::new();
    for base in bases {
        let module_base = root.join(base).join(&relative_module);
        candidates.push(module_base.with_extension("py"));
        candidates.push(module_base.join("__init__.py"));
    }

    candidates
        .into_iter()
        .filter_map(|candidate| candidate.strip_prefix(root).ok().map(Path::to_path_buf))
        .collect()
}

fn rust_candidates(root: &Path, source_file: &Path, target: &str) -> Vec<PathBuf> {
    let cleaned = target
        .trim()
        .trim_end_matches(';')
        .split('{')
        .next()
        .unwrap_or(target)
        .trim()
        .trim_end_matches("::");

    // Rust modules declared inside foo.rs are loaded from foo/name.rs.
    // Crate roots and mod.rs instead load their children from siblings.
    if !cleaned.contains("::") {
        let mut candidates = Vec::new();
        if let Some(parent) = source_file.parent() {
            let dir = match source_file.file_stem().and_then(|s| s.to_str()) {
                Some("lib" | "main" | "mod") | None => parent.to_path_buf(),
                Some(stem) => parent.join(stem),
            };
            add_rust_module_candidates(&mut candidates, &dir, cleaned);
        }
        return candidates;
    }

    let package_root = nearest_cargo_package_root(root, source_file);
    let (base, module_path, fallback_candidates) =
        if let Some(path) = cleaned.strip_prefix("crate::") {
            (
                package_root.join("src"),
                path,
                vec![
                    package_root.join("src/lib.rs"),
                    package_root.join("src/main.rs"),
                ],
            )
        } else if let Some(path) = cleaned.strip_prefix("self::") {
            (
                source_file
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .to_path_buf(),
                path,
                vec![source_file.to_path_buf()],
            )
        } else if cleaned.starts_with("super::") {
            let mut base = source_file
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf();
            let mut rest = cleaned;
            let mut first = true;
            while let Some(path) = rest.strip_prefix("super::") {
                if !first && !base.pop() {
                    return Vec::new();
                }
                first = false;
                rest = path;
            }
            let fallbacks = vec![base.with_extension("rs"), base.join("mod.rs")];
            (base, rest, fallbacks)
        } else {
            // External crate paths are not resolved as repository-local modules.
            return Vec::new();
        };

    let segments: Vec<&str> = module_path
        .split("::")
        .filter(|segment| !segment.is_empty() && *segment != "*")
        .collect();
    if segments.is_empty() {
        return Vec::new();
    }

    // The last segment may be an item rather than a module. Try successively
    // shorter prefixes. If none exists as a module, fall back to the parent
    // module file containing the imported item.
    let mut candidates = Vec::new();
    for length in (1..=segments.len()).rev() {
        let relative = segments[..length].iter().collect::<PathBuf>();
        add_rust_module_candidates(
            &mut candidates,
            &base,
            relative.as_os_str().to_string_lossy().as_ref(),
        );
    }

    if !candidates
        .iter()
        .any(|candidate| root.join(candidate).is_file())
    {
        candidates.extend(fallback_candidates);
    }
    candidates
}

fn nearest_cargo_package_root(root: &Path, source_file: &Path) -> PathBuf {
    let mut current = source_file.parent();
    while let Some(relative_dir) = current {
        if root.join(relative_dir).join("Cargo.toml").is_file() {
            return relative_dir.to_path_buf();
        }
        current = relative_dir.parent();
    }
    PathBuf::new()
}

fn add_rust_module_candidates(output: &mut Vec<PathBuf>, parent: &Path, module: &str) {
    let module_path = parent.join(module);
    output.push(module_path.with_extension("rs"));
    output.push(module_path.join("mod.rs"));
}

#[derive(Debug, Clone)]
struct GoModule {
    module_path: String,
    root: PathBuf,
}

fn discover_go_modules(root: &Path, source_files: &HashSet<PathBuf>) -> Vec<GoModule> {
    let mut roots = BTreeSet::new();
    for source in source_files {
        if source.extension().and_then(|value| value.to_str()) != Some("go") {
            continue;
        }

        let mut current = source.parent();
        while let Some(relative_dir) = current {
            let manifest = root.join(relative_dir).join("go.mod");
            if manifest.is_file() {
                roots.insert(relative_dir.to_path_buf());
                break;
            }
            current = relative_dir.parent();
        }
    }

    let mut modules = Vec::new();
    for module_root in roots {
        let manifest = root.join(&module_root).join("go.mod");
        let Ok(contents) = fs::read_to_string(manifest) else {
            continue;
        };
        let module_path = contents.lines().find_map(|line| {
            let line = line.split("//").next()?.trim();
            let value = line.strip_prefix("module ")?.trim();
            let value = value.trim_matches(['"', '\'']);
            (!value.is_empty()).then_some(value.to_owned())
        });

        if let Some(module_path) = module_path {
            modules.push(GoModule {
                module_path,
                root: module_root,
            });
        }
    }

    modules.sort_by_key(|module| std::cmp::Reverse(module.module_path.len()));
    modules
}

fn go_candidates(
    target: &str,
    modules: &[GoModule],
    source_files: &HashSet<PathBuf>,
) -> Vec<PathBuf> {
    let Some(module) = modules.iter().find(|module| {
        target == module.module_path
            || target
                .strip_prefix(&module.module_path)
                .is_some_and(|suffix| suffix.starts_with('/'))
    }) else {
        return Vec::new();
    };

    let package_suffix = target
        .strip_prefix(&module.module_path)
        .unwrap_or_default()
        .trim_start_matches('/');
    let package_dir = if package_suffix.is_empty() {
        module.root.clone()
    } else {
        module.root.join(package_suffix)
    };

    // A Go import refers to a package, which can consist of several files.
    // Return every indexed Go file in that package directory.
    source_files
        .iter()
        .filter(|path| {
            path.parent() == Some(package_dir.as_path())
                && path.extension().and_then(|value| value.to_str()) == Some("go")
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_typescript_imports_to_source_files() {
        let files = HashSet::from([
            PathBuf::from("src/main.ts"),
            PathBuf::from("src/components/button.tsx"),
        ]);

        let (resolution, paths) = resolve_import(
            Path::new("."),
            Path::new("src/main.ts"),
            SourceLanguage::TypeScript,
            "./components/button",
            &files,
            &[],
        );

        assert_eq!(resolution, ImportResolution::Local);
        assert_eq!(paths, vec![PathBuf::from("src/components/button.tsx")]);
    }

    #[test]
    fn leaves_package_imports_unresolved_instead_of_guessing() {
        let files = HashSet::from([PathBuf::from("src/main.ts")]);

        let (resolution, paths) = resolve_import(
            Path::new("."),
            Path::new("src/main.ts"),
            SourceLanguage::TypeScript,
            "react",
            &files,
            &[],
        );

        assert_eq!(resolution, ImportResolution::UnresolvedOrExternal);
        assert!(paths.is_empty());
    }

    #[test]
    fn skips_generated_and_hidden_directories() {
        assert!(should_skip_directory("node_modules"));
        assert!(should_skip_directory(".git"));
        assert!(should_skip_directory(".cache"));
        assert!(!should_skip_directory("src"));
    }
}

#[test]
fn same_file_symbol_match_wins_over_repository_wide_duplicates() {
    let local = Symbol {
        id: "local-parse".to_owned(),
        name: "parse".to_owned(),
        kind: crate::model::SymbolKind::Function,
        file_path: PathBuf::from("src/parser.rs"),
        start_byte: 20,
        end_byte: 40,
        start_line: 2,
        end_line: 3,
        parent_symbol_id: None,
    };
    let remote = Symbol {
        id: "remote-parse".to_owned(),
        name: "parse".to_owned(),
        kind: crate::model::SymbolKind::Function,
        file_path: PathBuf::from("src/other.rs"),
        start_byte: 10,
        end_byte: 30,
        start_line: 1,
        end_line: 2,
        parent_symbol_id: None,
    };
    let caller = Symbol {
        id: "caller".to_owned(),
        name: "run".to_owned(),
        kind: crate::model::SymbolKind::Function,
        file_path: PathBuf::from("src/parser.rs"),
        start_byte: 0,
        end_byte: 18,
        start_line: 1,
        end_line: 1,
        parent_symbol_id: None,
    };
    let symbols = [local, remote, caller];
    let by_name: HashMap<&str, Vec<&Symbol>> =
        symbols.iter().fold(HashMap::new(), |mut grouped, symbol| {
            grouped
                .entry(symbol.name.as_str())
                .or_default()
                .push(symbol);
            grouped
        });
    let by_id: HashMap<&str, &Symbol> = symbols
        .iter()
        .map(|symbol| (symbol.id.as_str(), symbol))
        .collect();
    let reference = RawReference {
        target_name: "parse".to_owned(),
        line: 1,
        from_symbol_id: Some("caller".to_owned()),
    };

    let (resolution, resolved) =
        resolve_symbol_reference(Path::new("src/parser.rs"), &reference, &by_name, &by_id);

    assert_eq!(resolution, SymbolResolution::SameFileNameMatch);
    assert_eq!(resolved.as_deref(), Some("local-parse"));
}

#[test]
fn enclosing_scope_match_wins_over_other_classes() {
    let scope_a = Symbol {
        id: "class-a".to_owned(),
        name: "Alpha".to_owned(),
        kind: crate::model::SymbolKind::Class,
        file_path: PathBuf::from("src/models.rs"),
        start_byte: 0,
        end_byte: 100,
        start_line: 1,
        end_line: 10,
        parent_symbol_id: None,
    };
    let scope_b = Symbol {
        id: "class-b".to_owned(),
        name: "Beta".to_owned(),
        kind: crate::model::SymbolKind::Class,
        file_path: PathBuf::from("src/models.rs"),
        start_byte: 101,
        end_byte: 200,
        start_line: 11,
        end_line: 20,
        parent_symbol_id: None,
    };
    let method_a = Symbol {
        id: "method-a".to_owned(),
        name: "validate".to_owned(),
        kind: crate::model::SymbolKind::Method,
        file_path: PathBuf::from("src/models.rs"),
        start_byte: 10,
        end_byte: 30,
        start_line: 2,
        end_line: 3,
        parent_symbol_id: Some("class-a".to_owned()),
    };
    let method_b = Symbol {
        id: "method-b".to_owned(),
        name: "validate".to_owned(),
        kind: crate::model::SymbolKind::Method,
        file_path: PathBuf::from("src/models.rs"),
        start_byte: 120,
        end_byte: 140,
        start_line: 12,
        end_line: 13,
        parent_symbol_id: Some("class-b".to_owned()),
    };
    let caller = Symbol {
        id: "caller-a".to_owned(),
        name: "check".to_owned(),
        kind: crate::model::SymbolKind::Method,
        file_path: PathBuf::from("src/models.rs"),
        start_byte: 31,
        end_byte: 50,
        start_line: 4,
        end_line: 5,
        parent_symbol_id: Some("class-a".to_owned()),
    };
    let symbols = [scope_a, scope_b, method_a, method_b, caller];
    let by_name: HashMap<&str, Vec<&Symbol>> =
        symbols.iter().fold(HashMap::new(), |mut grouped, symbol| {
            grouped
                .entry(symbol.name.as_str())
                .or_default()
                .push(symbol);
            grouped
        });
    let by_id: HashMap<&str, &Symbol> = symbols
        .iter()
        .map(|symbol| (symbol.id.as_str(), symbol))
        .collect();
    let reference = RawReference {
        target_name: "validate".to_owned(),
        line: 4,
        from_symbol_id: Some("caller-a".to_owned()),
    };

    let (resolution, resolved) =
        resolve_symbol_reference(Path::new("src/models.rs"), &reference, &by_name, &by_id);

    assert_eq!(resolution, SymbolResolution::ScopedNameMatch);
    assert_eq!(resolved.as_deref(), Some("method-a"));

    #[test]
    fn resolves_rust_file_backed_nested_module_paths() {
        let files = HashSet::from([
            PathBuf::from("src/lib.rs"),
            PathBuf::from("src/math.rs"),
            PathBuf::from("src/math/util.rs"),
        ]);
        let (from_root, root_files) = resolve_import(
            Path::new("."), Path::new("src/lib.rs"),
            SourceLanguage::Rust, "math", &files, &[],
        );
        assert_eq!(from_root, ImportResolution::Local);
        assert_eq!(root_files, vec![PathBuf::from("src/math.rs")]);
        let (nested, nested_files) = resolve_import(
            Path::new("."), Path::new("src/math.rs"),
            SourceLanguage::Rust, "util", &files, &[],
        );
        assert_eq!(nested, ImportResolution::Local);
        assert_eq!(nested_files, vec![PathBuf::from("src/math/util.rs")]);
    }
}
