use aggregator_utils::{
    orderbook::OrderbookState,
    types::{SwapRequest, SwapResponse},
};
use async_trait::async_trait;

/// A trait for processing incoming orderbook events. This is used to simulate incoming orderbook events to the aggregator.
/// This trait is not async to mimic the synchronous behavior of an an orderbook decoder, where latency and memory management
/// is most important.
pub trait EventProcessor: Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    fn process_orderbook(&self, new_orderbook: OrderbookState) -> Result<(), Self::Error>;
}

/// A trait for processing incoming requests. This is used to simulate incoming swap quote requests to the aggregator.
/// This trait is async to mimic the async behavior of an HTTP server.
#[async_trait]
pub trait RequestProcessor: Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn process_request(&self, request: SwapRequest) -> Result<SwapResponse, Self::Error>;
}
