mod adapter_ops;
mod cli;
mod cmd;
mod hook_support;
mod onboarding_surface;
mod render;
mod responses;
mod setup_support;

pub(crate) use responses::*;

use clap::Parser;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "thronglets=info".parse().unwrap()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = cli::Cli::parse();
    cmd::dispatch(cli).await;
}
