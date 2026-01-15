use clap::Parser;
use commands::Cli;
use tracing_subscriber::{
    filter::{FilterExt, LevelFilter},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Layer,
};

mod commands;
mod entry;

#[tokio::main]
async fn main() {
    init_tracing();
    let cli = Cli::parse();

    match entry::entry(cli).await {
        Ok(()) => {
            println!("Goodbye!");
        }
        Err(e) => {
            eprintln!("error: {:?}", e);
            std::process::exit(1);
        }
    }
}

/// Initializes tracing_subscriber with a default level of INFO
///
/// To override the log level, run this CLI with: RUST_LOG=aggregator=<log_level>
fn init_tracing() {
    let default_filters = vec![("aggregator", LevelFilter::INFO)];

    let mut default_filter = EnvFilter::new("");
    for (crate_name, level) in default_filters {
        default_filter =
            default_filter.add_directive(format!("{}={}", crate_name, level).parse().unwrap());
    }

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(""));
    let filter = default_filter.or(env_filter);

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .pretty()
                .with_filter(filter),
        )
        .init();
}
