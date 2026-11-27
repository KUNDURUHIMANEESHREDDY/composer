//! Compiler-gated multi-file Rust source integration.
use crate::{RustWiringError, RustWiringOptions, RustWiringResult, VerifiedCapsule};
use composer_intelligence::CapsuleFile;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};
use tree_sitter::{Node, Parser};

const MAX_FILES: usize = 32;
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Reconstruct an original module tree under a namespaced crate module.
/// Only invoked for disposable targets that must pass isolated Cargo tests.
pub(crate) fn wire_multifile_verified(
    options: &RustWiringOptions,
    verified: &VerifiedCapsule,
) -> Result<RustWiringResult, RustWiringError> {
    if !options.license_reviewed {
        return Err(RustWiringError::ReviewRequired);
    }
    let sources = verified
        .manifest
        .files
        .iter()
        .filter(|f| f.file_kind == "source")
        .collect::<Vec<_>>();
    if !(2..=MAX_FILES).contains(&sources.len()) {
        return Err(RustWiringError::Unsupported(
            "multi-file capsule requires 2-32 source files".into(),
        ));
    }
    let cap = &verified.manifest.capability_id;
    if cap.is_empty()
        || cap.len() > 64
        || !cap
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
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
        return Err(RustWiringError::Unsupported(
            "target must have src/lib.rs or src/main.rs".into(),
        ));
    };
    ensure_file(&entrypoint)?;
    let original = fs::read(&entrypoint)?;
    let original_text = std::str::from_utf8(&original)
        .map_err(|_| RustWiringError::Unsupported("target entrypoint is not UTF-8".into()))?;
    if original_text.contains(&declaration) || original_text.contains(&format!("mod {name};")) {
        return Err(RustWiringError::Unsupported(
            "module already declared".into(),
        ));
    }
    let root = target.join("src").join(&name);
    let backup = target.join("src").join(format!(".{name}.root-backup"));
    if fs::symlink_metadata(&root).is_ok() {
        return Err(RustWiringError::AlreadyExists(root));
    }
    if fs::symlink_metadata(&backup).is_ok() {
        return Err(RustWiringError::AlreadyExists(backup));
    }
    let mut modules = BTreeMap::<PathBuf, String>::new();
    let mut bytes_total = 0u64;
    let mut crate_root_seen = false;
    for entry in sources {
        let source = valid_relative_source(entry)?;
        let relative = if source == Path::new("lib.rs") || source == Path::new("main.rs") {
            if crate_root_seen {
                return Err(RustWiringError::Unsupported(
                    "two crate-root source files".into(),
                ));
            }
            crate_root_seen = true;
            PathBuf::from("mod.rs")
        } else {
            source.to_path_buf()
        };
        if modules.contains_key(&relative) {
            return Err(RustWiringError::Unsupported(format!(
                "duplicate module: {}",
                relative.display()
            )));
        }
        let file = verified.root.join(&entry.capsule_path);
        let metadata = fs::symlink_metadata(&file)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(RustWiringError::UnsafeTarget(file));
        }
        bytes_total = bytes_total.saturating_add(metadata.len());
        if bytes_total > MAX_BYTES {
            return Err(RustWiringError::Unsupported(
                "multi-file capsule exceeds 4 MiB".into(),
            ));
        }
        let text = fs::read_to_string(&file)
            .map_err(|_| RustWiringError::Unsupported("source file is not UTF-8".into()))?;
        for banned in [
            "include!",
            "include_str!",
            "include_bytes!",
            "#[path",
            "#[macro_export",
            "#[global_allocator",
            "#[no_mangle",
            "extern crate ",
        ] {
            if text.contains(banned) {
                return Err(RustWiringError::Unsupported(format!(
                    "unsupported source construct in multi-file wiring: {banned}"
                )));
            }
        }
        modules.insert(relative, rewrite_crate_paths(&text, &name)?);
    }
    // Reject ambiguous Rust folder/file forms of the same module.
    for path in modules.keys() {
        if path.file_name().and_then(|n| n.to_str()) == Some("mod.rs")
            && let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
            && modules.contains_key(&dir.with_extension("rs"))
        {
            return Err(RustWiringError::Unsupported(format!(
                "ambiguous Rust module path: {}",
                dir.display()
            )));
        }
    }
    // Derive complete ancestor and descendant module declarations.
    let paths = modules.keys().cloned().collect::<Vec<_>>();
    let mut children = BTreeMap::<PathBuf, BTreeSet<String>>::new();
    for path in paths {
        if path == Path::new("mod.rs") {
            continue;
        }
        let parent_dir = path.parent().unwrap_or_else(|| Path::new(""));
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| RustWiringError::Unsupported("non-UTF8 module file".into()))?;
        if stem != "mod" {
            validate_ident(stem)?;
            children
                .entry(parent_dir.to_path_buf())
                .or_default()
                .insert(stem.to_owned());
        }
        let mut ancestor = parent_dir.to_path_buf();
        while !ancestor.as_os_str().is_empty() {
            let child = ancestor
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| RustWiringError::Unsupported("non-UTF8 module directory".into()))?;
            validate_ident(child)?;
            let parent = ancestor.parent().unwrap_or_else(|| Path::new(""));
            children
                .entry(parent.to_path_buf())
                .or_default()
                .insert(child.to_owned());
            ancestor = parent.to_path_buf();
        }
    }
    for (dir, names) in children {
        let module_path = if dir.as_os_str().is_empty() {
            PathBuf::from("mod.rs")
        } else if modules.contains_key(&dir.with_extension("rs")) {
            dir.with_extension("rs")
        } else {
            dir.join("mod.rs")
        };
        let body = modules.entry(module_path).or_default();
        for child in names {
            if !declares_module(body, &child)? {
                body.push_str(&format!(
                    "\n// Software Composer: included module dependency\npub mod {child};\n"
                ));
            }
        }
    }
    if !modules.contains_key(Path::new("mod.rs")) {
        return Err(RustWiringError::Unsupported(
            "no module root created".into(),
        ));
    }
    fs::create_dir(&root)?;
    for (path, content) in modules {
        let dest = root.join(&path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&dest)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    let mut backup_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&backup)?;
    backup_file.write_all(&original)?;
    backup_file.sync_all()?;
    let mut update = original.clone();
    if !update.is_empty() && !update.ends_with(b"\n") {
        update.push(b'\n');
    }
    update.extend_from_slice(
        format!(
            "\n// Software Composer: verified multi-file capsule {}\n{declaration}\n",
            verified.manifest.capsule_id
        )
        .as_bytes(),
    );
    fs::write(&entrypoint, &update)?;
    if fs::read(&entrypoint)? != update {
        return Err(RustWiringError::Unsupported(
            "entrypoint write/readback mismatch".into(),
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

fn valid_relative_source(entry: &CapsuleFile) -> Result<&Path, RustWiringError> {
    let source = entry
        .source_path
        .as_deref()
        .ok_or_else(|| RustWiringError::Unsupported("missing original source path".into()))?;
    let relative = source.strip_prefix("src").map_err(|_| {
        RustWiringError::Unsupported(format!("source not under root src/: {}", source.display()))
    })?;
    if relative.as_os_str().is_empty()
        || relative.extension().and_then(|s| s.to_str()) != Some("rs")
        || !relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(RustWiringError::Unsupported(
            "invalid source module path".into(),
        ));
    }
    for component in relative.components() {
        if let Component::Normal(item) = component
            && item.to_str().is_none()
        {
            return Err(RustWiringError::Unsupported("non-UTF8 module path".into()));
        }
    }
    Ok(relative)
}

fn validate_ident(value: &str) -> Result<(), RustWiringError> {
    if value.is_empty()
        || value.as_bytes()[0].is_ascii_digit()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(RustWiringError::Unsupported(format!(
            "invalid module identifier: {value}"
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
fn rust_tree(source: &str) -> Result<tree_sitter::Tree, RustWiringError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|err| RustWiringError::Unsupported(format!("Rust parser init failed: {err}")))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| RustWiringError::Unsupported("Rust parsing failed".into()))?;
    if tree.root_node().has_error() {
        return Err(RustWiringError::Unsupported(
            "invalid Rust source syntax".into(),
        ));
    }
    Ok(tree)
}
fn walk_nodes(node: Node<'_>, f: &mut impl FnMut(Node<'_>)) {
    f(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_nodes(child, f);
    }
}

/// Modify compiler-identified crate-root path tokens, never comments/strings.
fn rewrite_crate_paths(source: &str, namespace: &str) -> Result<String, RustWiringError> {
    let tree = rust_tree(source)?;
    let mut replacements = Vec::<usize>::new();
    walk_nodes(tree.root_node(), &mut |node| {
        if node.child_count() == 0
            && node.utf8_text(source.as_bytes()).ok() == Some("crate")
            && (node.kind() == "crate" || node.kind() == "identifier")
            && source[node.end_byte()..].trim_start().starts_with("::")
        {
            replacements.push(node.end_byte());
        }
    });
    replacements.sort_unstable();
    replacements.dedup();
    let mut out = source.to_owned();
    for pos in replacements.into_iter().rev() {
        out.insert_str(pos, &format!("::{namespace}"));
    }
    Ok(out)
}

fn declares_module(source: &str, name: &str) -> Result<bool, RustWiringError> {
    if source.is_empty() {
        return Ok(false);
    }
    let tree = rust_tree(source)?;
    let mut found = false;
    walk_nodes(tree.root_node(), &mut |node| {
        if node.kind() == "mod_item" {
            let code = &source[node.start_byte()..node.end_byte()];
            if code.trim_end().ends_with(';')
                && code
                    .split_whitespace()
                    .any(|word| word == name || word == format!("{name};"))
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
    fn rewrites_paths_but_not_comments_or_strings() {
        let input = "// crate::foo\nconst S: &str = \"crate::foo\";\nuse crate::helper::answer;\npub fn x() -> u32 { crate::helper::answer() }\n";
        let out = rewrite_crate_paths(input, "composer_new").unwrap();
        assert!(out.contains("use crate::composer_new::helper::answer;"));
        assert!(out.contains("crate::composer_new::helper::answer()"));
        assert!(out.contains("// crate::foo"));
        assert!(out.contains("\"crate::foo\""));
    }
    #[test]
    fn refuses_unsafe_source_paths() {
        assert!(rewrite_crate_paths("fn x( {", "foo").is_err());
        assert!(validate_ident("../foo").is_err());
        assert!(validate_ident("1abc").is_err());
    }
}
