//! Compiler-verified Rust module-tree transplantation for disposable targets.
//! Unverified wire-rust still refuses multi-file sources.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};
use composer_intelligence::CapsuleFile;
use tree_sitter::{Node, Parser};
use crate::{RustWiringError, RustWiringOptions, RustWiringResult, VerifiedCapsule};

const MAX_MODULE_FILES: usize = 32;
const MAX_MODULE_BYTES: u64 = 4 * 1024 * 1024;

/// Rebuild an upstream module graph under one private composer_<capability>
/// namespace and adjust compiler-identified crate-root paths. Publication
/// requires Docker Cargo compilation and tests in the verified caller.
pub(crate) fn wire_multifile_verified(
    options: &RustWiringOptions,
    verified: &VerifiedCapsule,
) -> Result<RustWiringResult, RustWiringError> {
    if !options.license_reviewed {
        return Err(RustWiringError::ReviewRequired);
    }
    let source_entries = verified.manifest.files.iter()
        .filter(|entry| entry.file_kind == "source").collect::<Vec<_>>();
    if !(2..=MAX_MODULE_FILES).contains(&source_entries.len()) {
        return Err(RustWiringError::Unsupported("multi-file capsules require 2-32 source files".into()));
    }
    let cap = &verified.manifest.capability_id;
    if cap.is_empty() || cap.len() > 64 ||
        !cap.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err(RustWiringError::Unsupported("invalid capability ID".into()));
    }
    let name = format!("composer_{}", cap.replace('-', "_"));
    let target = options.target_root.canonicalize()?;
    ensure_directory(&target.join("src"))?;
    ensure_file(&target.join("Cargo.toml"))?;
    let lib = target.join("src/lib.rs");
    let main = target.join("src/main.rs");
    let (entrypoint, declaration) = if lib.is_file() {
        (lib, format!("pub mod {name};"))
    } else if main.is_file() {
        (main, format!("mod {name};"))
    } else {
        return Err(RustWiringError::Unsupported("target requires src/lib.rs or src/main.rs".into()));
    };
    ensure_file(&entrypoint)?;
    let original = fs::read(&entrypoint)?;
    let original_text = std::str::from_utf8(&original).map_err(|_| {
        RustWiringError::Unsupported("target entrypoint is not UTF-8".into())
    })?;
    if original_text.contains(&declaration) || original_text.contains(&format!("mod {name};")) {
        return Err(RustWiringError::Unsupported("target already declares this capability".into()));
    }
    let root = target.join("src").join(&name);
    let backup = target.join("src").join(format!(".{name}.root-backup"));
    if fs::symlink_metadata(&root).is_ok() {
        return Err(RustWiringError::AlreadyExists(root));
    }
    if fs::symlink_metadata(&backup).is_ok() {
        return Err(RustWiringError::AlreadyExists(backup));
    }

    // All source files must be verified capsule artifacts; paths may not
    // escape root src. Rejection occurs before writing the target.
    let mut modules = BTreeMap::<PathBuf, String>::new();
    let mut source_bytes = 0u64;
    let mut crate_root_seen = false;
    for entry in source_entries {
        let original_relative = valid_source_path(entry)?;
        let relative = if original_relative == Path::new("lib.rs") ||
            original_relative == Path::new("main.rs")
        {
            if crate_root_seen {
                return Err(RustWiringError::Unsupported("multiple crate-root source files".into()));
            }
            crate_root_seen = true;
            PathBuf::from("mod.rs")
        } else {
            original_relative.to_path_buf()
        };
        if modules.contains_key(&relative) {
            return Err(RustWiringError::Unsupported(format!(
                "duplicate Rust destination: {}", relative.display()
            )));
        }
        let source_file = verified.root.join(&entry.capsule_path);
        let metadata = fs::symlink_metadata(&source_file)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(RustWiringError::UnsafeTarget(source_file));
        }
        source_bytes = source_bytes.saturating_add(metadata.len());
        if source_bytes > MAX_MODULE_BYTES {
            return Err(RustWiringError::Unsupported("source files exceed 4 MiB".into()));
        }
        let source_text = fs::read_to_string(&source_file).map_err(|_| {
            RustWiringError::Unsupported("source is not UTF-8".into())
        })?;
        for unsupported in [
            "include!", "include_str!", "include_bytes!", "#[path",
            "#[macro_export", "#[global_allocator", "#[no_mangle", "extern crate "
        ] {
            if source_text.contains(unsupported) {
                return Err(RustWiringError::Unsupported(format!(
                    "unsupported path-sensitive source construct: {unsupported}"
                )));
            }
        }
        modules.insert(relative, rewrite_crate_paths(&source_text, &name)?);
    }
    for path in modules.keys() {
        if path.file_name().and_then(|n| n.to_str()) == Some("mod.rs") &&
            let Some(dir) = path.parent() &&
            !dir.as_os_str().is_empty() &&
            modules.contains_key(&dir.with_extension("rs"))
        {
            return Err(RustWiringError::Unsupported(format!(
                "ambiguous Rust module file and folder: {}",dir.display()
            )));
        }
    }
    // File layout defines an ancestor graph. A missing intermediate mod.rs is
    // generated; missing declarations in existing parents are appended.
    let source_paths = modules.keys().cloned().collect::<Vec<_>>();
    let mut declarations = BTreeMap::<PathBuf, BTreeSet<String>>::new();
    for source_path in source_paths {
        if source_path == Path::new("mod.rs") { continue; }
        let parent = source_path.parent().unwrap_or_else(|| Path::new(""));
        let stem = source_path.file_stem().and_then(|v| v.to_str()).ok_or_else(|| {
            RustWiringError::Unsupported("invalid module filename".into())
        })?;
        if stem != "mod" {
            validate_ident(stem)?;
            declarations.entry(parent.to_path_buf()).or_default().insert(stem.to_owned());
        }
        let mut ancestor = parent.to_path_buf();
        while !ancestor.as_os_str().is_empty() {
            let component = ancestor.file_name().and_then(|v| v.to_str()).ok_or_else(|| {
                RustWiringError::Unsupported("invalid directory module".into())
            })?;
            validate_ident(component)?;
            let outer = ancestor.parent().unwrap_or_else(|| Path::new(""));
            declarations.entry(outer.to_path_buf()).or_default().insert(component.to_owned());
            ancestor = outer.to_path_buf();
        }
    }
    for (directory, modules_to_declare) in declarations {
        let file = if directory.as_os_str().is_empty() {
            PathBuf::from("mod.rs")
        } else if modules.contains_key(&directory.with_extension("rs")) {
            directory.with_extension("rs")
        } else {
            directory.join("mod.rs")
        };
        let contents = modules.entry(file).or_default();
        for child in modules_to_declare {
            if !declares_module(contents, &child)? {
                contents.push_str(&format!(
                    "\n// Software Composer: imported module closure\npub mod {child};\n"
                ));
            }
        }
    }
    if !modules.contains_key(Path::new("mod.rs")) {
        return Err(RustWiringError::Unsupported("could not generate module root".into()));
    }

    // This destination is exclusively a disposable copy. The verified caller
    // deletes the entire staging directory if file writes or Docker tests fail.
    fs::create_dir(&root)?;
    for (relative, source) in modules {
        let destination = root.join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create_new(true).write(true)
            .open(&destination)?;
        file.write_all(source.as_bytes())?;
        file.sync_all()?;
    }
    let mut backup_file = OpenOptions::new().create_new(true).write(true)
        .open(&backup)?;
    backup_file.write_all(&original)?;
    backup_file.sync_all()?;
    let mut modified = original.clone();
    if !modified.is_empty() && !modified.ends_with(b"\n") {
        modified.push(b'\n');
    }
    modified.extend_from_slice(format!(
        "\n// Software Composer: verified multi-file capsule {}\n{declaration}\n",
        verified.manifest.capsule_id
    ).as_bytes());
    fs::write(&entrypoint, &modified)?;
    if fs::read(&entrypoint)? != modified {
        return Err(RustWiringError::Unsupported(
            "entrypoint post-write verification failed".into()
        ));
    }
    Ok(RustWiringResult {
        module_name: name,
        source_module: root.join("mod.rs"),
        entrypoint,
        entrypoint_backup: backup,
        capsule_id: verified.manifest.capsule_id.clone(),
        build_or_test_commands_executed: false,
        status: "multi_file_staged_pending_independent_compiler_tests".into(),
    })
}

fn valid_source_path(entry: &CapsuleFile) -> Result<&Path, RustWiringError> {
    let source = entry.source_path.as_deref().ok_or_else(|| {
        RustWiringError::Unsupported("capsule source path is missing".into())
    })?;
    let relative = source.strip_prefix("src").map_err(|_| {
        RustWiringError::Unsupported("only root-package src modules supported".into())
    })?;
    if relative.as_os_str().is_empty() ||
        relative.extension().and_then(|v| v.to_str()) != Some("rs") ||
        !relative.components().all(|v| matches!(v, Component::Normal(_)))
    {
        return Err(RustWiringError::Unsupported(format!(
            "unsafe source module path: {}", source.display()
        )));
    }
    if relative.components().any(|part| match part {
        Component::Normal(value) => value.to_str().is_none(),
        _ => true,
    }) {
        return Err(RustWiringError::Unsupported("non-UTF8 module path".into()));
    }
    Ok(relative)
}
fn validate_ident(value: &str) -> Result<(), RustWiringError> {
    if value.is_empty() || value.as_bytes()[0].is_ascii_digit() ||
        !value.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(RustWiringError::Unsupported(format!(
            "invalid Rust module identifier: {value}"
        )));
    }
    Ok(())
}
fn ensure_file(path: &Path) -> Result<(), RustWiringError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(RustWiringError::UnsafeTarget(path.to_path_buf()));
    }
    Ok(())
}
fn ensure_directory(path: &Path) -> Result<(), RustWiringError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(RustWiringError::UnsafeTarget(path.to_path_buf()));
    }
    Ok(())
}
fn parse_rust(source: &str) -> Result<tree_sitter::Tree, RustWiringError> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into()).map_err(|err| {
        RustWiringError::Unsupported(format!("Rust grammar unavailable: {err}"))
    })?;
    let tree = parser.parse(source, None).ok_or_else(|| {
        RustWiringError::Unsupported("Rust parser returned no tree".into())
    })?;
    if tree.root_node().has_error() {
        return Err(RustWiringError::Unsupported("invalid Rust source syntax".into()));
    }
    Ok(tree)
}
fn walk_nodes(node: Node<'_>, visitor: &mut impl FnMut(Node<'_>)) {
    visitor(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_nodes(child, visitor);
    }
}
/// AST-based byte edits avoid modifying strings and comments.
fn rewrite_crate_paths(source: &str, namespace: &str) -> Result<String, RustWiringError> {
    let tree = parse_rust(source)?;
    let mut positions = Vec::new();
    walk_nodes(tree.root_node(), &mut |node| {
        if node.child_count() == 0 &&
            node.utf8_text(source.as_bytes()).ok() == Some("crate") &&
            (node.kind() == "crate" || node.kind() == "identifier") &&
            source[node.end_byte()..].trim_start().starts_with("::")
        {
            positions.push(node.end_byte());
        }
    });
    positions.sort_unstable();
    positions.dedup();
    let mut result = source.to_owned();
    for position in positions.into_iter().rev() {
        result.insert_str(position, &format!("::{namespace}"));
    }
    Ok(result)
}
fn declares_module(source: &str, name: &str) -> Result<bool, RustWiringError> {
    if source.is_empty() { return Ok(false); }
    let tree = parse_rust(source)?;
    let mut found = false;
    walk_nodes(tree.root_node(), &mut |node| {
        if node.kind() == "mod_item" {
            let text = &source[node.start_byte()..node.end_byte()];
            if text.trim_end().ends_with(';') &&
                text.split_whitespace().any(|word|
                    word == name || word == format!("{name};"))
            {
                found = true;
            }
        }
    });
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rewrites_crate_paths_without_touching_comments_or_literals() {
        let source = "// crate::example\nconst S: &str = \"crate::example\";\nuse crate::math::add;\npub fn compute() -> u32 { crate::math::add() }\n";
        let adapted = rewrite_crate_paths(source, "composer_example").unwrap();
        assert!(adapted.contains("use crate::composer_example::math::add;"));
        assert!(adapted.contains("crate::composer_example::math::add()"));
        assert!(adapted.contains("// crate::example"));
        assert!(adapted.contains("\"crate::example\""));
    }
    #[test]
    fn refuses_invalid_module_names_and_syntax() {
        assert!(validate_ident("2bad").is_err());
        assert!(validate_ident("../invalid").is_err());
        assert!(rewrite_crate_paths("fn x( {", "composer_a").is_err());
    }
}
