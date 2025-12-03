use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use aggregator_utils::orderbook::OrderbookState;
use aggregator_utils::types::{Address, Quantity, Swap};
use tracing::debug;

use crate::takehome::format_duration;
use crate::takehome::matching::match_order;
use crate::takehome::state::{AggregatorState, GraphEdge};

// Bounds search complexity. 3 hops covers most real DEX routes
// (e.g. USDC -> ETH -> DOGE -> GTE). This is a reasonable compromise between
// search space and complexity.
const MAX_HOPS: usize = 3;

/// Snapshot of orderbook state for consistent routing.
/// Taking a snapshot ensures BFS sees a consistent view even as orderbooks update.
struct RoutingSnapshot {
    orderbooks: HashMap<(Address, Address), OrderbookState>,
    graph_edges: HashMap<Address, Vec<GraphEdge>>,
}

impl RoutingSnapshot {
    fn from_state(state: &AggregatorState) -> Self {
        let mut orderbooks = HashMap::new();
        // scc uses scan with FnMut closure
        state.orderbooks.scan(|k, v| {
            orderbooks.insert(*k, v.clone());
        });

        let mut graph_edges = HashMap::new();
        state.graph_edges.scan(|k, v| {
            graph_edges.insert(*k, v.clone());
        });

        Self {
            orderbooks,
            graph_edges,
        }
    }

    fn neighbors(&self, token: &Address) -> &[GraphEdge] {
        self.graph_edges
            .get(token)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn get_orderbook(&self, pair: &(Address, Address)) -> Option<&OrderbookState> {
        self.orderbooks.get(pair)
    }
}

// Find route that maximizes output. Uses BFS to explore all paths up to MAX_HOPS.
pub fn find_best_route(
    state: &AggregatorState,
    input_token: Address,
    output_token: Address,
    input_amount: Quantity,
) -> Option<Vec<Swap>> {
    // Take snapshot for consistent routing
    let start = Instant::now();
    let snapshot = RoutingSnapshot::from_state(state);
    debug!(time = %format_duration(start.elapsed()), "snapshot created");

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
        for edge in snapshot.neighbors(&current_token) {
            // Get the orderbook from snapshot using pair key
            let Some(book) = snapshot.get_orderbook(&edge.pair) else {
                continue;
            };

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
