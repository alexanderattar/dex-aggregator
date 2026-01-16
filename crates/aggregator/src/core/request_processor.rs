use aggregator_utils::types::{SwapRequest, SwapResponse, SwapResponseSuccess};
use async_trait::async_trait;
use tracing::{info, warn};

use crate::core::routing::find_best_route;
use crate::core::state::SharedState;
use crate::traits::RequestProcessor;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("routing failed: {0}")]
    RoutingFailed(String),
}

#[derive(Debug, Clone)]
pub struct DexRequestProcessor {
    state: SharedState,
}

impl DexRequestProcessor {
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

        if request.input_token == request.output_token {
            warn!(token = %request.input_token, "swap rejected: same token");
            return Ok(SwapResponse::Failure("same token".to_string()));
        }

        if request.input_amount == 0 {
            warn!("swap rejected: zero amount");
            return Ok(SwapResponse::Failure("zero amount".to_string()));
        }

        let input_known = self.state.contains_token(request.input_token);
        let output_known = self.state.contains_token(request.output_token);

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
            self.state
                .metrics
                .requests_failed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(SwapResponse::Failure(reason.to_string()));
        }

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
                    "swap rejected: no route found"
                );
                self.state
                    .metrics
                    .requests_failed
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(SwapResponse::Failure("no route".to_string()));
            }
        };

        let output = route.last().map(|s| s.expected_output_amount).unwrap_or(0);

        if output < request.min_output_amount {
            warn!(
                output = output,
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

        Ok(SwapResponse::Success(SwapResponseSuccess { route }))
    }
}
