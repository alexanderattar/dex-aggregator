use std::time::Duration;

use aggregator::{
    backend::{
        event_thread::{EventThread, EventThreadConfig},
        request_thread::{RequestThread, RequestThreadConfig},
    },
    core::{
        event_processor::DexEventProcessor, request_processor::DexRequestProcessor,
        state::create_shared_state,
    },
};
use aggregator_utils::types::Address;
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemBuilder, Toplevel};

use crate::commands::{Cli, Command, RunCommandArgs};

pub async fn entry(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Run(args) => run(args).await,
    }
}

/// Constructs all subcomponents and starts the event loops.
///
/// Should you want to add another thread, it should be defined here as a
/// subsystem similar to backend/event_thread.rs or backend/request_thread.rs
async fn run(args: RunCommandArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mut tokens = Vec::with_capacity(args.num_tokens as usize);
    for _ in 0..args.num_tokens {
        tokens.push(Address::new_random());
    }

    // Create unique token pairs
    let mut token_pairs = Vec::with_capacity(args.num_tokens as usize);
    'outer: for i in 0..args.num_tokens {
        for j in i + 1..args.num_tokens {
            token_pairs.push((tokens[i as usize], tokens[j as usize]));

            if token_pairs.len() == args.num_orderbooks as usize {
                break 'outer;
            }
        }
    }

    let shared_orderbooks = create_shared_state();
    let event_processor = DexEventProcessor::new(shared_orderbooks.clone());
    let request_processor = DexRequestProcessor::new(shared_orderbooks);

    // RequestThread is a thread that simulates incoming swap requests to the aggregator
    let request_thread = RequestThread::new(
        RequestThreadConfig {
            requests_per_second: args.requests_per_second,
            slippage_tolerance_bps: args.slippage_tolerance_bps,
            tokens,
        },
        request_processor,
    );

    // EventThread is a thread that simulates incoming orderbook events to the aggregator
    let event_thread = EventThread::new(
        EventThreadConfig {
            events_per_second: args.events_per_second,
            num_prices: args.num_price_levels as usize,
            pairs: token_pairs,
        },
        event_processor,
    );

    // The main event loop.
    //
    // This loop will run until the user presses SIGINT
    // Any additional threads should be added here as subsystems
    Toplevel::new(|s| async move {
        s.start(SubsystemBuilder::new(
            "event_thread",
            event_thread.into_subsystem(),
        ));

        s.start(SubsystemBuilder::new(
            "request_thread",
            request_thread.into_subsystem(),
        ));
    })
    .catch_signals()
    .handle_shutdown_requests(Duration::from_secs(10))
    .await
    .map_err(Into::into)
}
