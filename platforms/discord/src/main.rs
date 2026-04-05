use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod worker;
use worker::DiscordWorker;

#[tokio::main]
async fn main() -> Result<()> {
    match dotenvy::dotenv() {
        Ok(path) => info!(path = %path.display(), "Loaded .env file"),
        Err(dotenvy::Error::Io(_)) => info!("No .env file found"),
        Err(e) => tracing::warn!(error = %e, "Failed to parse .env file"),
    }

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();

    let token = std::env::var("DISCORD_BOT_TOKEN").unwrap_or_default();

    info!("=== Discord Service Starting ===");

    let mut worker = DiscordWorker::new(token);

    // Catch shutdown
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("Shutdown signal received");
        let _ = tx.send(()).await;
    });

    tokio::select! {
        res = worker.run() => {
            if let Err(e) = res {
                tracing::error!(error = %e, "Worker failed");
            }
        }
        _ = rx.recv() => {
            // shutting down
        }
    }

    info!("=== Discord Service Stopped ===");
    Ok(())
}
