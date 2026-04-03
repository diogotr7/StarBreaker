use clap::Parser;
use rmcp::ServiceExt;

mod tools;

#[derive(Parser)]
#[command(name = "starbreaker-mcp", about = "MCP server for Star Citizen game data")]
struct Cli {
    /// Path to Data.p4k (overrides auto-discovery)
    #[arg(long, env = "SC_DATA_P4K")]
    p4k: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .init();

    let cli = Cli::parse();

    // Start the MCP server immediately — data loads lazily on first tool call.
    // This avoids the 14s P4k/DataCore load blocking the MCP handshake.
    let handler = tools::StarBreakerMcp::new(cli.p4k);
    log::info!("MCP server starting (data loads on first tool call)");

    let transport = rmcp::transport::io::stdio();
    let server = handler.serve(transport).await?;
    server.waiting().await?;

    Ok(())
}
