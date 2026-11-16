use std::{error::Error, fs, net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};
use composer_core::ProjectSpec;

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
        Command::Serve { bind } => {
            println!("Starting Software Composer API on {bind}");
            composer_api::serve(bind).await?;
        }
    }

    Ok(())
}
