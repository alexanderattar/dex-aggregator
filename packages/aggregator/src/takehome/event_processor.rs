use aggregator_utils::orderbook::OrderbookState;
use tracing::trace;

use crate::takehome::state::SharedState;
use crate::traits::EventProcessor;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("state lock failed")]
    LockFailed,
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

    // Ingest orderbook updates. Write lock held briefly for upsert only.
    fn process_orderbook(&self, orderbook: OrderbookState) -> Result<(), Self::Error> {
        trace!(
            base = %orderbook.base_token,
            quote = %orderbook.quote_token,
            "orderbook update"
        );

        // Update shared state with new orderbook
        let mut state = self.state.write().map_err(|_| Error::LockFailed)?;
        state.upsert_orderbook(orderbook);
        Ok(())
    }
}
