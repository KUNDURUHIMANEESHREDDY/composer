use std::env;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("composer_api=info")),
        )
        .init();

    let bind = env::var("COMPOSER_BIND")
        .unwrap_or_else(|_| "127.0.0.1:3000".to_owned())
        .parse()?;

    tracing::info!(address = %bind, "API server starting");
    composer_api::serve(bind).await?;
    Ok(())
}
