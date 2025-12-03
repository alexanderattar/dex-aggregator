use std::time::Instant;

use aggregator_utils::orderbook::OrderbookState;
use tracing::{debug, trace};

use crate::takehome::format_duration;
use crate::takehome::state::SharedState;
use crate::traits::EventProcessor;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid orderbook: {0}")]
    InvalidOrderbook(String),
}

#[derive(Debug, Clone)]
pub struct TakehomeEventProcessor {
    state: SharedState,
}

impl TakehomeEventProcessor {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

impl EventProcessor for TakehomeEventProcessor {
    type Error = Error;

    fn process_orderbook(&self, orderbook: OrderbookState) -> Result<(), Self::Error> {
        trace!(
            base = %orderbook.base_token,
            quote = %orderbook.quote_token,
            "orderbook update"
        );

        let start = Instant::now();

        self.state.upsert_orderbook(orderbook);

        debug!(
            time = %format_duration(start.elapsed()),
            "orderbook updated"
        );

        Ok(())
    }
}
