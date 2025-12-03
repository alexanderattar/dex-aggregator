use aggregator_utils::types::{SwapRequest, SwapResponse, SwapResponseSuccess};
use async_trait::async_trait;
use tracing::{info, warn};

use crate::takehome::routing::find_best_route;
use crate::takehome::state::SharedState;
use crate::traits::RequestProcessor;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("routing failed: {0}")]
    RoutingFailed(String),
}

#[derive(Debug, Clone)]
pub struct TakehomeRequestProcessor {
    state: SharedState,
}

impl TakehomeRequestProcessor {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl RequestProcessor for TakehomeRequestProcessor {
    type Error = Error;

    async fn process_request(&self, request: SwapRequest) -> Result<SwapResponse, Self::Error> {
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

        // Check tokens exist before searching (better error messages)
        let input_known = self.state.contains_token(request.input_token);
        let output_known = self.state.contains_token(request.output_token);

        // Reject swaps with unknown tokens
        if !input_known || !output_known {
            let reason = match (input_known, output_known) {
                (false, false) => "unknown tokens",
                (false, true) => "unknown input token",
                (true, false) => "unknown output token",
                _ => unreachable!(),
            };
            warn!(
                input_token = %request.input_token,
                output_token = %request.output_token,
                "swap rejected: {reason}"
            );
            return Ok(SwapResponse::Failure(reason.to_string()));
        }

        // Find best route by maximizing output
        // Snapshot is taken inside find_best_route for consistent routing
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

        Ok(SwapResponse::Success(SwapResponseSuccess { route }))
    }
}
