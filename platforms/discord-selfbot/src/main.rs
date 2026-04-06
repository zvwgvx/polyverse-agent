use anyhow::Result;
use tracing::{info, warn, error};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    match dotenvy::dotenv() {
        Ok(path) => info!(path = %path.display(), "Loaded .env file"),
        Err(dotenvy::Error::Io(_)) => info!("No .env file found"),
        Err(e) => warn!(error = %e, "Failed to parse .env file"),
    }

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();

    let token = std::env::var("DISCORD_SELFBOT_TOKEN").unwrap_or_default();
    if token.is_empty() {
        warn!("DISCORD_SELFBOT_TOKEN not set, exiting");
        return Ok(());
    }

    info!("=== Discord Selfbot Runner Starting ===");

    let node_script_path = concat!(env!("CARGO_MANIFEST_DIR"), "/nodejs-selfbot/index.js");

    let mut child = match tokio::process::Command::new("node")
        .arg(node_script_path)
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => {
            info!("Spawned Node.js selfbot process (PID: {:?})", child.id());
            child
        }
        Err(e) => {
            error!(error = %e, "Failed to spawn Node.js selfbot process");
            return Err(e.into());
        }
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("Shutdown signal received");
        let _ = tx.send(()).await;
    });

    tokio::select! {
        status = child.wait() => {
            match status {
                Ok(s) => info!("Node.js process exited with status: {}", s),
                Err(e) => error!("Node.js process wait error: {}", e),
            }
        }
        _ = rx.recv() => {
            info!("Killing Node.js process...");
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }

    info!("=== Discord Selfbot Runner Stopped ===");
    Ok(())
}
