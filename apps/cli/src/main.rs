use std::{error::Error, fs, net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};
use composer_core::ProjectSpec;
use composer_integration::{
    BatchRustOptions, IntegrationOptions, RustWiringOptions, UpstreamCrateOptions,
    UpstreamRecoveryCandidate, UpstreamRecoveryOptions, VerifiedRustOptions, apply_integration,
    compose_verified_rust, compose_verified_rust_batch, discover_rust_api_contracts,
    plan_integration, recover_upstream_crate, verify_capsule, verify_upstream_crate,
    wire_rust_module,
};
use composer_intelligence::{
    CapsuleOptions, analyze_repository, create_feature_capsule, recommend_feature_seeds,
};

#[derive(Debug, Parser)]
#[command(
    name = "composer",
    version,
    about = "Autonomous software composition toolkit"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a JSON project specification before starting a composition run.
    ValidateSpec {
        /// Path to a project specification JSON file.
        path: PathBuf,
    },
    /// Clone and inspect a public GitHub repository without running its code.
    Inspect {
        /// HTTPS URL of a public GitHub repository.
        repository_url: String,
        /// Parent directory for isolated repository checkouts.
        #[arg(long, default_value = ".composer/sources")]
        workspace: PathBuf,
    },
    /// Parse repository source and emit the symbol/import dependency graph.
    Analyze {
        /// Path to a previously acquired or local Git repository.
        repository: PathBuf,
        /// Optional destination for the full machine-readable graph.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Rank likely implementation symbols for a requested capability.
    Recommend {
        /// Path to an analyzed source repository.
        repository: PathBuf,
        /// Stable capability identifier, e.g. authentication.
        #[arg(long)]
        capability_id: String,
        /// Human-readable capability requirement.
        #[arg(long)]
        description: String,
        /// Maximum number of ranked seed candidates.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Build a Feature Capsule from selected symbol seeds and static dependencies.
    Capsule {
        /// Path to an analyzed source repository.
        repository: PathBuf,
        /// Stable capability identifier, e.g. authentication.
        #[arg(long)]
        capability_id: String,
        /// Human-readable capability requirement.
        #[arg(long)]
        description: String,
        /// One or more symbol IDs selected from an analysis/recommendation result.
        #[arg(long = "seed-symbol", required = true)]
        seed_symbols: Vec<String>,
        /// New output directory; existing directories are never overwritten.
        #[arg(long)]
        output: PathBuf,
    },
    /// Independently verify every file in a Feature Capsule against its manifest.
    VerifyCapsule {
        /// Path to a Feature Capsule directory.
        capsule: PathBuf,
    },
    /// Compare a verified Feature Capsule with a target repository.
    Compatibility {
        /// Path to a Feature Capsule directory.
        capsule: PathBuf,
        /// Target application repository.
        target: PathBuf,
        /// Optional path for the compatibility plan JSON.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Suggest Rust API mappings for explicit, unfinished public target functions.
    /// Read-only: suggestions require separate review and compiler verification.
    SuggestRustApi {
        /// Verified Feature Capsule directory.
        capsule: PathBuf,
        /// Target standalone library root with src/lib.rs containing todo!() stubs.
        target: PathBuf,
        /// Optional path to save machine-readable JSON recommendations.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Stage a verified Feature Capsule into a new target-local namespace.
    Integrate {
        /// Path to a Feature Capsule directory.
        capsule: PathBuf,
        /// Target application repository.
        target: PathBuf,
        /// New path inside the target repository; defaults to .composer/integrations/<capability-id>.
        #[arg(long)]
        destination: Option<PathBuf>,
    },
    /// Opt-in source wiring for a single reviewed, standalone Rust module.
    WireRust {
        /// Verified Feature Capsule path.
        capsule: PathBuf,
        /// Target Rust crate root (must contain src/lib.rs or src/main.rs).
        target: PathBuf,
        /// Assert independent license and source review; does not certify safety.
        #[arg(long)]
        license_reviewed: bool,
    },
    /// Resolve direct Rust dependencies and verify wired code with offline Docker Cargo tests.
    VerifyRust {
        /// Verified Feature Capsule directory.
        capsule: PathBuf,
        /// Original target Rust application; never modified.
        target: PathBuf,
        /// New output directory for successfully verified source and lockfile.
        #[arg(long)]
        output: PathBuf,
        /// Assert independent license and source review; not legal clearance.
        #[arg(long)]
        license_reviewed: bool,
        /// Read-only Cargo registry cache to use for offline dependency solving.
        #[arg(long)]
        registry_cache: Option<PathBuf>,
        /// Repeat source::public_item=target_api_name for compiler-verified aliases.
        #[arg(long = "api-alias")]
        api_aliases: Vec<String>,
        /// Repeat source::fn=target_fn(u16)->u64; only infallible Into wrappers.
        #[arg(long = "api-signature")]
        api_signatures: Vec<String>,
        /// Exact suggestion manually reviewed from suggest-rust-api output.
        #[arg(long = "reviewed-api-rule")]
        reviewed_api_rule: Option<String>,
        /// Pre-existing independent test that calls the target stub.
        #[arg(long = "contract-test")]
        contract_test: Option<PathBuf>,
    },
    /// Reuse a published crate from an independently reviewed public Rust repository.
    /// Cargo resolves exact registry artifacts; Composer never regenerates upstream source.
    VerifyExternalRustCrate {
        /// Original standalone Cargo library; never modified.
        target: PathBuf,
        /// New output directory, published only after isolated offline tests pass.
        #[arg(long)]
        output: PathBuf,
        /// Published crates.io package name.
        #[arg(long)]
        package: String,
        /// Exact published version (not a semver range).
        #[arg(long)]
        version: String,
        /// Existing upstream public function to re-export.
        #[arg(long)]
        public_symbol: String,
        /// Target-visible API name referenced by the contract test.
        #[arg(long)]
        target_alias: String,
        /// Existing independent target-owned tests/<name>.rs contract.
        #[arg(long)]
        contract_test: PathBuf,
        /// Pre-provisioned Cargo registry; mounted read-only inside Docker.
        #[arg(long)]
        registry_cache: PathBuf,
        /// Human-reviewed public upstream GitHub repository URL (provenance claim).
        #[arg(long)]
        upstream_repository: String,
        /// Human-reviewed 40-character commit SHA (not proven to match crate artifact).
        #[arg(long)]
        upstream_commit: String,
        /// Confirm independent license/source review; not legal clearance.
        #[arg(long)]
        license_reviewed: bool,
    },
    /// Try 1-5 reviewed external package candidates in preference order.
    /// Resolves cached semver versions, falls back after incorrect behavior,
    /// and isolates existing dependency conflicts under Cargo aliases.
    RecoverExternalRustCrate {
        /// Original target Cargo project. Never modified.
        target: PathBuf,
        /// New verified output directory.
        #[arg(long)]
        output: PathBuf,
        /// JSON array of reviewed external package candidates.
        #[arg(long)]
        candidates_json: PathBuf,
        /// Target-visible function that the behavioral contract exercises.
        #[arg(long)]
        target_alias: String,
        /// Existing independent target-owned test in tests/<name>.rs.
        #[arg(long)]
        contract_test: PathBuf,
        /// Pre-populated crates.io registry cache, never fetched by Composer.
        #[arg(long)]
        registry_cache: PathBuf,
        /// Confirm independent license/source review for all candidates.
        #[arg(long)]
        license_reviewed: bool,
    },
    /// Verify 2-5 distinct Rust Feature Capsules in sequence, with rollback on any failure.
    VerifyRustBatch {
        /// Two to five reviewed Feature Capsules; repeat this flag for each.
        #[arg(long = "capsule", required = true, num_args = 2..=5)]
        capsules: Vec<PathBuf>,
        /// Original target Rust application; never modified.
        #[arg(long)]
        target: PathBuf,
        /// New output directory published only after every integration passes.
        #[arg(long)]
        output: PathBuf,
        /// Assert independent source and license review for every capsule.
        #[arg(long)]
        license_reviewed: bool,
        /// Read-only Cargo registry cache for offline dependency solving.
        #[arg(long)]
        registry_cache: Option<PathBuf>,
    },
    /// Start the local composition API.
    Serve {
        /// Address on which the API listens.
        #[arg(long, default_value = "127.0.0.1:3000")]
        bind: SocketAddr,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::ValidateSpec { path } => {
            let contents = fs::read_to_string(&path)?;
            let spec: ProjectSpec = serde_json::from_str(&contents)?;
            spec.validate()?;

            println!("Project specification is valid: {}", spec.name);
            println!("{}", serde_json::to_string_pretty(&spec)?);
        }
        Command::Inspect {
            repository_url,
            workspace,
        } => {
            let inspection = composer_repository::acquire_and_inspect(&repository_url, &workspace)?;
            println!("{}", serde_json::to_string_pretty(&inspection)?);
        }
        Command::Analyze { repository, output } => {
            let graph = analyze_repository(&repository)?;
            if let Some(output) = output {
                fs::write(&output, serde_json::to_vec_pretty(&graph)?)?;
                println!(
                    "Analysis written to {}\n{}",
                    output.display(),
                    serde_json::to_string_pretty(&graph.summary)?
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&graph)?);
            }
        }
        Command::Recommend {
            repository,
            capability_id,
            description,
            limit,
        } => {
            let graph = analyze_repository(&repository)?;
            let candidates = recommend_feature_seeds(&graph, &capability_id, &description, limit);
            println!("{}", serde_json::to_string_pretty(&candidates)?);
        }
        Command::Capsule {
            repository,
            capability_id,
            description,
            seed_symbols,
            output,
        } => {
            let graph = analyze_repository(&repository)?;
            let result = create_feature_capsule(
                &graph,
                &CapsuleOptions {
                    capability_id,
                    description,
                    seed_symbol_ids: seed_symbols,
                    output_dir: output,
                },
            )?;
            println!(
                "Feature Capsule created at {}\n{}",
                result.output_dir.display(),
                serde_json::to_string_pretty(&result.manifest)?
            );
        }
        Command::VerifyCapsule { capsule } => {
            let result = verify_capsule(&capsule)?;
            println!("{}", serde_json::to_string_pretty(&result.verification)?);
        }
        Command::Compatibility {
            capsule,
            target,
            output,
        } => {
            let plan = plan_integration(&capsule, &target)?;
            if let Some(output) = output {
                fs::write(&output, serde_json::to_vec_pretty(&plan)?)?;
                println!(
                    "Compatibility plan written to {}\nlevel={:?} strategy={:?}",
                    output.display(),
                    plan.level,
                    plan.strategy
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
        }
        Command::Integrate {
            capsule,
            target,
            destination,
        } => {
            let result = apply_integration(&IntegrationOptions {
                capsule_root: capsule,
                target_root: target,
                destination,
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::WireRust {
            capsule,
            target,
            license_reviewed,
        } => {
            let result = wire_rust_module(&RustWiringOptions {
                capsule_root: capsule,
                target_root: target,
                license_reviewed,
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::SuggestRustApi {
            capsule,
            target,
            output,
        } => {
            let report = discover_rust_api_contracts(&capsule, &target)?;
            let serialized = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                // Never overwrite an existing report.
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                use std::io::Write;
                file.write_all(serialized.as_bytes())?;
            }
            println!("{serialized}");
        }
        Command::VerifyRust {
            capsule,
            target,
            output,
            license_reviewed,
            registry_cache,
            api_aliases,
            api_signatures,
            reviewed_api_rule,
            contract_test,
        } => {
            let result = compose_verified_rust(&VerifiedRustOptions {
                capsule_root: capsule,
                target_root: target,
                output_root: output,
                license_reviewed,
                registry_cache,
                api_aliases,
                api_signatures,
                reviewed_api_rule,
                contract_test,
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::VerifyExternalRustCrate {
            target,
            output,
            package,
            version,
            public_symbol,
            target_alias,
            contract_test,
            registry_cache,
            upstream_repository,
            upstream_commit,
            license_reviewed,
        } => {
            let result = verify_upstream_crate(&UpstreamCrateOptions {
                target_root: target,
                output_root: output,
                package,
                version,
                public_symbol,
                target_alias,
                contract_test,
                registry_cache,
                upstream_repository,
                upstream_commit,
                license_reviewed,
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::RecoverExternalRustCrate {
            target,
            output,
            candidates_json,
            target_alias,
            contract_test,
            registry_cache,
            license_reviewed,
        } => {
            let metadata = fs::symlink_metadata(&candidates_json)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 64 * 1024 {
                return Err("candidate list must be a regular JSON file no larger than 64 KiB".into());
            }
            let candidates: Vec<UpstreamRecoveryCandidate> =
                serde_json::from_slice(&fs::read(candidates_json)?)?;
            let report = recover_upstream_crate(&UpstreamRecoveryOptions {
                target_root: target,
                output_root: output,
                target_alias,
                contract_test,
                registry_cache,
                candidates,
                license_reviewed,
            })?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::VerifyRustBatch {
            capsules,
            target,
            output,
            license_reviewed,
            registry_cache,
        } => {
            let result = compose_verified_rust_batch(&BatchRustOptions {
                capsule_roots: capsules,
                target_root: target,
                output_root: output,
                license_reviewed,
                registry_cache,
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Serve { bind } => {
            println!("Starting Software Composer API on {bind}");
            composer_api::serve(bind).await?;
        }
    }

    Ok(())
}
