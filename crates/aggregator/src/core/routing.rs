use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use aggregator_utils::orderbook::OrderbookState;
use aggregator_utils::types::{Address, Quantity, Swap};
use tracing::debug;

use crate::core::format_duration;
use crate::core::matching::match_order;
use crate::core::state::{AggregatorState, GraphEdge, OrderbookHealth};

const MAX_HOPS: usize = 3;
const STALE_TTL: Duration = Duration::from_secs(30);

/// Snapshot of orderbook state for consistent routing.
struct RoutingSnapshot {
    orderbooks: HashMap<(Address, Address), OrderbookState>,
    graph_edges: HashMap<Address, Vec<GraphEdge>>,
    health: HashMap<(Address, Address), OrderbookHealth>,
}

impl RoutingSnapshot {
    fn from_state(state: &AggregatorState) -> Self {
        let mut orderbooks = HashMap::new();
        state.orderbooks.scan(|k, v| {
            orderbooks.insert(*k, v.clone());
        });

        let mut graph_edges = HashMap::new();
        state.graph_edges.scan(|k, v| {
            graph_edges.insert(*k, v.clone());
        });

        let mut health = HashMap::new();
        state.orderbook_health.scan(|k, v| {
            health.insert(*k, v.clone());
        });

        state
            .metrics
            .snapshots_taken
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Self {
            orderbooks,
            graph_edges,
            health,
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

    fn is_usable(&self, pair: &(Address, Address)) -> bool {
        self.health
            .get(pair)
            .map(|h| h.has_valid_spread && h.last_updated.elapsed() <= STALE_TTL)
            .unwrap_or(false)
    }
}

/// Find route that maximizes output using BFS.
pub fn find_best_route(
    state: &AggregatorState,
    input_token: Address,
    output_token: Address,
    input_amount: Quantity,
) -> Option<Vec<Swap>> {
    let start = Instant::now();
    let snapshot = RoutingSnapshot::from_state(state);
    debug!(time = %format_duration(start.elapsed()), "snapshot created");

    let mut queue = VecDeque::new();
    queue.push_back((input_token, input_amount, Vec::new()));

    let mut best_output = 0u64;
    let mut best_route: Option<Vec<Swap>> = None;

    while let Some((current_token, current_amount, hops)) = queue.pop_front() {
        if current_token == output_token && !hops.is_empty() {
            if current_amount > best_output {
                best_output = current_amount;
                best_route = Some(hops);
            }
            continue;
        }

        if hops.len() >= MAX_HOPS {
            continue;
        }

        for edge in snapshot.neighbors(&current_token) {
            if !snapshot.is_usable(&edge.pair) {
                state
                    .metrics
                    .stale_or_unhealthy_skipped
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }

            let Some(book) = snapshot.get_orderbook(&edge.pair) else {
                continue;
            };

            let result = match_order(book, edge.side, current_amount);

            if result.output_produced == 0 {
                continue;
            }

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
