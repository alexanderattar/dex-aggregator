use aggregator_utils::types::{SwapRequest, SwapResponse, SwapResponseSuccess};
use async_trait::async_trait;
use tracing::{info, warn};

use crate::core::routing::find_best_route;
use crate::core::state::SharedState;
use crate::traits::RequestProcessor;

/// Errors that can occur during request processing.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("routing failed: {0}")]
    RoutingFailed(String),
}

/// Processes swap requests by finding optimal multi-hop routes.
///
/// The request processor enforces rate limiting, circuit breaker protection,
/// and slippage tolerance before executing swaps. It uses BFS-based routing
/// to find the best path across multiple orderbooks.
///
/// # Features
///
/// - Rate limiting to prevent abuse
/// - Circuit breaker for graceful degradation under failures
/// - Slippage protection based on user-specified minimum output
/// - Multi-hop routing (up to 3 hops) for optimal execution
///
/// # Example
///
/// ```ignore
/// let state = create_shared_state();
/// let processor = DexRequestProcessor::new(state);
/// let response = processor.process_request(swap_request).await?;
/// ```
#[derive(Debug, Clone)]
pub struct DexRequestProcessor {
    state: SharedState,
}

impl DexRequestProcessor {
    /// Create a new request processor with the given shared state.
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl RequestProcessor for DexRequestProcessor {
    type Error = Error;

    async fn process_request(&self, request: SwapRequest) -> Result<SwapResponse, Self::Error> {
        self.state
            .metrics
            .requests_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Rate limit check: reject if too many requests
        if !self.state.rate_limiter.try_acquire() {
            warn!("swap rejected: rate limited");
            self.state
                .metrics
                .requests_rate_limited
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(SwapResponse::Failure("rate limited".to_string()));
        }

        // Circuit breaker check: reject early if system is degraded
        if !self.state.circuit_breaker.allow_request() {
            warn!(
                circuit_state = %self.state.circuit_breaker.current_state(),
                "swap rejected: circuit breaker open"
            );
            self.state
                .metrics
                .requests_failed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(SwapResponse::Failure("service degraded".to_string()));
        }

        // Reject swaps with the same token
        if request.input_token == request.output_token {
            warn!(token = %request.input_token, "swap rejected: same token");
            return Ok(SwapResponse::Failure("same token".to_string()));
        }

        // Reject swaps with zero amount
        if request.input_amount == 0 {
            warn!(
                input_token = %request.input_token,
                output_token = %request.output_token,
                "swap rejected: zero amount"
            );
            return Ok(SwapResponse::Failure("zero amount".to_string()));
        }

        // Check tokens exist before searching
        let input_known = self.state.contains_token(request.input_token);
        let output_known = self.state.contains_token(request.output_token);

        // Reject swaps with unknown tokens (infrastructure issue, affects circuit breaker)
        // Return generic error to client but log details server-side
        if !input_known || !output_known {
            let detail = match (input_known, output_known) {
                (false, false) => "both tokens unknown",
                (false, true) => "input token unknown",
                (true, false) => "output token unknown",
                _ => unreachable!(),
            };
            warn!(
                input_token = %request.input_token,
                output_token = %request.output_token,
                detail,
                "swap rejected: invalid request"
            );
            self.state
                .metrics
                .requests_failed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.state.circuit_breaker.record_failure();
            // Generic error to avoid leaking token existence info
            return Ok(SwapResponse::Failure("invalid request".to_string()));
        }

        // Find best route (infrastructure issue if fails, affects circuit breaker)
        let route = match find_best_route(
            &self.state,
            request.input_token,
            request.output_token,
            request.input_amount,
        ) {
            Some(r) => r,
            None => {
                warn!(
                    input_token = %request.input_token,
                    output_token = %request.output_token,
                    input_amount = request.input_amount,
                    "swap rejected: no path"
                );
                self.state
                    .metrics
                    .requests_failed
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.state.circuit_breaker.record_failure();
                return Ok(SwapResponse::Failure("no path".to_string()));
            }
        };

        let output = route.last().map(|s| s.expected_output_amount).unwrap_or(0);

        // Slippage protection: reject if output is below user's minimum
        if output < request.min_output_amount {
            warn!(
                input_token = %request.input_token,
                output_token = %request.output_token,
                input_amount = request.input_amount,
                output_amount = output,
                min_output = request.min_output_amount,
                "swap rejected: slippage"
            );
            self.state
                .metrics
                .requests_failed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(SwapResponse::Failure(format!(
                "slippage: {} < {}",
                output, request.min_output_amount
            )));
        }

        info!(
            input_token = %request.input_token,
            output_token = %request.output_token,
            input_amount = request.input_amount,
            output_amount = output,
            hops = route.len(),
            "route found"
        );

        // Successful route resets circuit breaker failure count
        self.state.circuit_breaker.record_success();

        Ok(SwapResponse::Success(SwapResponseSuccess { route }))
    }
}
