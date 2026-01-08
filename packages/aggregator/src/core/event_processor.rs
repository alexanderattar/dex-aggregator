use std::time::Instant;

use aggregator_utils::orderbook::OrderbookState;
use tracing::{debug, trace};

use crate::core::format_duration;
use crate::core::state::{AggregatorState, SharedState};
use crate::traits::EventProcessor;

/// Errors that can occur during event processing.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid orderbook: {0}")]
    InvalidOrderbook(String),
}

/// Processes incoming orderbook events and updates the shared aggregator state.
///
/// The event processor validates each orderbook update (checking for crossed spreads
/// and empty books) before ingesting it into the state. Invalid orderbooks are
/// rejected and tracked in metrics.
///
/// # Example
///
/// ```ignore
/// let state = create_shared_state();
/// let processor = DexEventProcessor::new(state);
/// processor.process_orderbook(orderbook)?;
/// ```
#[derive(Debug, Clone)]
pub struct DexEventProcessor {
    state: SharedState,
}

impl DexEventProcessor {
    /// Create a new event processor with the given shared state.
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

impl EventProcessor for DexEventProcessor {
    type Error = Error;

    fn process_orderbook(&self, orderbook: OrderbookState) -> Result<(), Self::Error> {
        trace!(
            base = %orderbook.base_token,
            quote = %orderbook.quote_token,
            "orderbook update"
        );

        let start = Instant::now();

        // Validate orderbook before ingesting
        if !AggregatorState::validate_orderbook(&orderbook) {
            self.state
                .metrics
                .events_invalid
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Err(Error::InvalidOrderbook("crossed/empty book".into()));
        }

        self.state.upsert_orderbook(orderbook);

        self.state
            .metrics
            .events_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        debug!(
            time = %format_duration(start.elapsed()),
            "orderbook updated"
        );

        Ok(())
    }
}
