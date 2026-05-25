use anyhow::Result;
use clap::Parser;
use tracing_subscriber::fmt::writer::MakeWriterExt;

mod tools;
mod validation;

#[derive(Parser)]
#[command(name = "surrealmcp", about = "Minimal MCP server for SurrealDB")]
struct Cli {
    /// SurrealDB endpoint URL (e.g. wss://..., ws://localhost:8000)
    #[arg(long, env = "SURREAL_ENDPOINT")]
    endpoint: String,

    /// Default namespace
    #[arg(long, env = "SURREAL_NS")]
    ns: String,

    /// Default database
    #[arg(long, env = "SURREAL_DB")]
    db: String,

    /// Username for authentication (optional)
    #[arg(long, env = "SURREAL_USER")]
    user: Option<String>,

    /// Password for authentication (optional)
    #[arg(long, env = "SURREAL_PASS")]
    pass: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Install rustls crypto provider (surrealdb 3.x uses aws-lc-rs)
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();

    // Set up file logging to ~/.surreal-mcp/logs/
    let home = std::env::var("HOME").expect("HOME environment variable not set");
    let log_dir = std::path::PathBuf::from(home)
        .join(".surreal-mcp")
        .join("logs");
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "surrealmcp.log");
    let writer = file_appender.and(std::io::stderr);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(writer)
        .init();

    let cli = Cli::parse();

    tracing::debug!(
        user = cli.user.as_deref().unwrap_or("<not set>"),
        pass_present = cli.pass.is_some(),
        "Auth credentials"
    );

    let endpoint = &cli.endpoint;
    let ns = &cli.ns;
    let db_name = &cli.db;
    tracing::info!(%endpoint, "Connecting to SurrealDB");
    let db = surrealdb::engine::any::connect(endpoint).await?;
    tracing::debug!("Connected, setting ns/db");
    db.use_ns(ns).use_db(db_name).await?;
    tracing::debug!("ns/db set");
    if let (Some(user), Some(pass)) = (cli.user.clone(), cli.pass.clone()) {
        db.signin(surrealdb::opt::auth::Root {
            username: user.clone(),
            password: pass,
        })
        .await?;
        tracing::info!(%user, "Authenticated");
    }

    tracing::info!(%ns, db = %db_name, "Connected, starting MCP server on stdio");

    let service = tools::SurrealMcp::new(db, cli.ns, cli.db);
    let server = rmcp::serve_server(service, (tokio::io::stdin(), tokio::io::stdout())).await?;
    server.waiting().await?;

    Ok(())
}
