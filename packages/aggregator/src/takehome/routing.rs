use std::collections::VecDeque;

use aggregator_utils::types::{Address, Quantity, Swap};

use crate::takehome::matching::match_order;
use crate::takehome::state::AggregatorState;

// Bounds search complexity. 3 hops covers most real DEX routes
// (e.g. USDC -> ETH -> DOGE -> GTE). This is a reasonable compromise between
// search space and complexity.
const MAX_HOPS: usize = 3;

// Find route that maximizes output. Uses BFS to explore all paths up to MAX_HOPS.
pub fn find_best_route(
    state: &AggregatorState,
    input_token: Address,
    output_token: Address,
    input_amount: Quantity,
) -> Option<Vec<Swap>> {
    // (current token, amount we have, path taken)
    let mut queue = VecDeque::new();
    queue.push_back((input_token, input_amount, Vec::new()));

    // Track best output at destination separately since we can't compare
    // amounts across different tokens mid-search
    let mut best_output = 0u64;
    let mut best_route: Option<Vec<Swap>> = None;

    while let Some((current_token, current_amount, hops)) = queue.pop_front() {
        // Reached destination? Check if it's the best route so far
        if current_token == output_token && !hops.is_empty() {
            if current_amount > best_output {
                best_output = current_amount;
                best_route = Some(hops);
            }
            continue; // don't explore past destination
        }

        // The request processor will return a no path error if the route is too long
        // This limits the search space
        if hops.len() >= MAX_HOPS {
            continue;
        }

        // Try all tokens reachable in one trade
        for edge in state.graph.neighbors(current_token) {
            // Get the orderbook for the current edge and match the order
            let book = &state.orderbooks[edge.orderbook_idx];
            let result = match_order(book, edge.side, current_amount);

            // Skip if no liquidity on this path
            if result.output_produced == 0 {
                continue;
            }

            // Create the next hop and add it to the queue
            let mut next_hops = hops.clone();
            next_hops.push(Swap {
                input_token: current_token,
                output_token: edge.target,
                direction: edge.side,
                input_amount: result.input_consumed,
                expected_output_amount: result.output_produced,
            });

            queue.push_back((edge.target, result.output_produced, next_hops));
        }
    }

    best_route
}
