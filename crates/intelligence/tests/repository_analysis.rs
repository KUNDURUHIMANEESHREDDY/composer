use std::{fs, path::PathBuf};

use composer_intelligence::{
    CapsuleOptions, ImportResolution, SourceLanguage, analyze_repository, create_feature_capsule,
};
use uuid::Uuid;

fn temporary_workspace() -> PathBuf {
    std::env::temp_dir().join(format!("composer-analysis-{}", Uuid::new_v4()))
}

#[test]
fn analyzes_multilanguage_repository_and_builds_transitive_capsule() {
    let workspace = temporary_workspace();
    let repository = workspace.join("source");
    fs::create_dir_all(repository.join("src")).unwrap();
    fs::create_dir_all(repository.join("web")).unwrap();
    fs::create_dir_all(repository.join("python")).unwrap();
    fs::create_dir_all(repository.join("go")).unwrap();

    fs::write(
        repository.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(repository.join("LICENSE"), "MIT License\n").unwrap();

    fs::write(
        repository.join("src/lib.rs"),
        "mod auth;\npub fn authenticate_user() { auth::verify_token(); }\n",
    )
    .unwrap();
    fs::write(repository.join("src/auth.rs"), "pub fn verify_token() {}\n").unwrap();

    fs::write(
        repository.join("web/app.ts"),
        "import { signIn } from './auth';\nexport function loginUser() { signIn(); }\n",
    )
    .unwrap();
    fs::write(
        repository.join("web/auth.ts"),
        "export function signIn() {}\n",
    )
    .unwrap();

    fs::write(
        repository.join("python/auth.py"),
        "from tokens import verify_python_token\ndef authenticate_python_user():\n    verify_python_token()\n",
    )
    .unwrap();
    fs::write(
        repository.join("python/tokens.py"),
        "def verify_python_token():\n    return True\n",
    )
    .unwrap();

    fs::write(
        repository.join("go/auth.go"),
        "package auth\nimport \"context\"\nfunc AuthenticateGoUser(ctx context.Context) error { return nil }\n",
    )
    .unwrap();

    let graph = analyze_repository(&repository).unwrap();

    assert!(graph.summary.languages.contains(&SourceLanguage::Rust));
    assert!(
        graph
            .summary
            .languages
            .contains(&SourceLanguage::TypeScript)
    );
    assert!(graph.summary.languages.contains(&SourceLanguage::Python));
    assert!(graph.summary.languages.contains(&SourceLanguage::Go));
    assert!(graph.summary.symbols_found >= 7);
    assert!(
        graph.summary.local_imports_resolved >= 3,
        "expected local imports to resolve; import graph: {:#?}",
        graph.imports
    );

    let rust_seed = graph
        .symbols
        .iter()
        .find(|symbol| symbol.name == "authenticate_user")
        .expect("Rust seed symbol should be indexed");

    let capsule = create_feature_capsule(
        &graph,
        &CapsuleOptions {
            capability_id: "authentication".to_owned(),
            description: "Authenticate users and verify tokens".to_owned(),
            seed_symbol_ids: vec![rust_seed.id.clone()],
            output_dir: workspace.join("capsule"),
        },
    )
    .unwrap();

    assert!(
        capsule
            .manifest
            .included_source_files
            .contains(&PathBuf::from("src/lib.rs"))
    );
    assert!(
        capsule
            .manifest
            .included_source_files
            .contains(&PathBuf::from("src/auth.rs"))
    );
    assert!(
        capsule
            .manifest
            .license_evidence_files
            .contains(&PathBuf::from("LICENSE"))
    );
    assert!(capsule.manifest.license_review_required);
    assert!(capsule.output_dir.join("source/src/auth.rs").is_file());
    assert!(capsule.output_dir.join("manifest.json").is_file());

    let go_context = graph
        .imports
        .iter()
        .find(|item| item.target == "context")
        .expect("Go import should be extracted");
    assert_eq!(
        go_context.resolution,
        ImportResolution::UnresolvedOrExternal
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn resolves_rust_modules_using_nested_cargo_package_roots() {
    let workspace = temporary_workspace();
    let package = workspace.join("serde_core");
    fs::create_dir_all(package.join("src/ser")).unwrap();
    fs::write(
        package.join("Cargo.toml"),
        "[package]\nname = \"serde_core\"\n",
    )
    .unwrap();
    fs::write(package.join("src/lib.rs"), "pub mod ser;\n").unwrap();
    fs::write(
        package.join("src/ser/mod.rs"),
        "pub use self::impossible::Impossible;\nuse crate::lib::*;\n",
    )
    .unwrap();
    fs::write(
        package.join("src/ser/impossible.rs"),
        "pub struct Impossible;\n",
    )
    .unwrap();
    fs::write(
        package.join("src/ser/impls.rs"),
        "use super::SerializeStruct;\npub fn build() {}\n",
    )
    .unwrap();

    let graph = analyze_repository(&workspace).unwrap();
    let sibling_import = graph
        .imports
        .iter()
        .find(|dependency| dependency.target == "self::impossible::Impossible")
        .expect("self module import should be parsed");
    assert_eq!(sibling_import.resolution, ImportResolution::Local);
    assert_eq!(
        sibling_import.resolved_file,
        Some(PathBuf::from("serde_core/src/ser/impossible.rs"))
    );

    let crate_import = graph
        .imports
        .iter()
        .find(|dependency| dependency.target == "crate::lib::*")
        .expect("crate-root import should be parsed");
    assert_eq!(crate_import.resolution, ImportResolution::Local);
    assert_eq!(
        crate_import.resolved_file,
        Some(PathBuf::from("serde_core/src/lib.rs"))
    );

    let super_import = graph
        .imports
        .iter()
        .find(|dependency| dependency.target == "super::SerializeStruct")
        .expect("super item import should be parsed");
    assert_eq!(super_import.resolution, ImportResolution::Local);
    assert_eq!(
        super_import.resolved_file,
        Some(PathBuf::from("serde_core/src/ser/mod.rs"))
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn resolves_go_module_imports_to_all_files_in_a_local_package() {
    let workspace = temporary_workspace();
    let repository = workspace.join("go-project");
    fs::create_dir_all(repository.join("cmd")).unwrap();
    fs::create_dir_all(repository.join("internal/auth")).unwrap();

    fs::write(
        repository.join("go.mod"),
        "module example.com/composer\ngo 1.22\n",
    )
    .unwrap();
    fs::write(
        repository.join("cmd/main.go"),
        "package main\nimport \"example.com/composer/internal/auth\"\nfunc main() { auth.Start() }\n",
    )
    .unwrap();
    fs::write(
        repository.join("internal/auth/auth.go"),
        "package auth\nfunc Start() {}\n",
    )
    .unwrap();
    fs::write(
        repository.join("internal/auth/token.go"),
        "package auth\nfunc Verify() bool { return true }\n",
    )
    .unwrap();

    let graph = analyze_repository(&repository).unwrap();
    let dependency = graph
        .imports
        .iter()
        .find(|item| item.target == "example.com/composer/internal/auth")
        .expect("local Go module import should be present");

    assert_eq!(dependency.resolution, ImportResolution::Local);
    assert_eq!(
        dependency.resolved_files,
        vec![
            PathBuf::from("internal/auth/auth.go"),
            PathBuf::from("internal/auth/token.go")
        ]
    );

    let _ = fs::remove_dir_all(workspace);
}
