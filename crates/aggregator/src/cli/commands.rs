use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Run(RunCommandArgs),
    // Add any other commands here if needed
}

#[derive(Debug, Args)]
pub struct RunCommandArgs {
    #[arg(
        long,
        global = true,
        default_value = "32",
        help = "The number of tokens to simulate"
    )]
    pub num_tokens: u64,

    #[arg(
        long,
        global = true,
        default_value = "10",
        help = "The number of orderbooks to simulate"
    )]
    pub num_orderbooks: u64,

    #[arg(
        long,
        global = true,
        default_value = "100",
        help = "The number of events to simulate per second"
    )]
    pub events_per_second: u32,

    #[arg(
        long,
        global = true,
        default_value = "10",
        help = "The number of swap requests to simulate per second"
    )]
    pub requests_per_second: u32,

    #[arg(long, global = true, default_value = "50")]
    pub slippage_tolerance_bps: u16,

    #[arg(long, global = true, default_value = "10")]
    pub num_price_levels: u16,
}
