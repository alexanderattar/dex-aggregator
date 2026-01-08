use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use aggregator_utils::orderbook::OrderbookState;
use aggregator_utils::types::{Address, Quantity, Swap};
use tracing::debug;

use crate::core::format_duration;
use crate::core::matching::match_order;
use crate::core::state::{AggregatorState, BookMeta, GraphEdge};

// Bounds search complexity. 3 hops covers most real DEX routes
// (e.g. USDC -> ETH -> DOGE -> SHIB). This is a reasonable compromise between
// search space and complexity.
const MAX_HOPS: usize = 3;
// Skip books older than this
const STALE_TTL: Duration = Duration::from_secs(30);

/// Snapshot of orderbook state for consistent routing.
/// Taking a snapshot ensures BFS sees a consistent view even as orderbooks update.
/// Uses Arc-wrapped HashMaps for O(1) cloning when shared across requests.
#[derive(Clone)]
struct RoutingSnapshot {
    orderbooks: Arc<HashMap<(Address, Address), OrderbookState>>,
    graph_edges: Arc<HashMap<Address, Vec<GraphEdge>>>,
    meta: Arc<HashMap<(Address, Address), BookMeta>>,
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

        let mut meta = HashMap::new();
        state.orderbook_meta.scan(|k, v| {
            meta.insert(*k, v.clone());
        });

        state
            .metrics
            .snapshots_taken
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Self {
            orderbooks: Arc::new(orderbooks),
            graph_edges: Arc::new(graph_edges),
            meta: Arc::new(meta),
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
        self.meta
            .get(pair)
            .map(|m| m.healthy && m.updated_at.elapsed() <= STALE_TTL)
            .unwrap_or(false)
    }
}

/// Arena node for path reconstruction without per-edge cloning.
struct PathNode {
    swap: Swap,
    parent: Option<usize>, // Index into arena, None for first hop
}

/// Reconstruct path from arena by walking parent pointers.
fn reconstruct_path(arena: &[PathNode], mut idx: usize) -> Vec<Swap> {
    let mut path = Vec::with_capacity(MAX_HOPS);
    loop {
        path.push(arena[idx].swap.clone());
        match arena[idx].parent {
            Some(parent_idx) => idx = parent_idx,
            None => break,
        }
    }
    path.reverse();
    path
}

/// Finds the optimal multi-hop trading route between two tokens.
///
/// Uses BFS to explore all paths up to `MAX_HOPS` (3) and returns the route
/// that maximizes output amount. Takes a point-in-time snapshot of orderbook
/// state for consistent routing.
///
/// # Arguments
///
/// * `state` - The aggregator state containing orderbooks and graph topology
/// * `input_token` - The token being sold
/// * `output_token` - The token being bought
/// * `input_amount` - Amount of input token to swap
///
/// # Returns
///
/// `Some(Vec<Swap>)` containing the optimal route, or `None` if no path exists.
///
/// # Performance
///
/// Uses arena allocation for path tracking to avoid O(h*e) cloning overhead
/// where h = hops and e = edges explored.
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

    // Arena for path nodes to avoid cloning paths on every edge
    let mut arena: Vec<PathNode> = Vec::new();

    // (current token, amount we have, hop count, parent index in arena or None)
    let mut queue: VecDeque<(Address, u64, usize, Option<usize>)> = VecDeque::new();
    queue.push_back((input_token, input_amount, 0, None));

    // Track best output at destination
    let mut best_output = 0u64;
    let mut best_path_end: Option<usize> = None;

    while let Some((current_token, current_amount, hop_count, parent_idx)) = queue.pop_front() {
        // Reached destination? Check if it's the best route so far
        if current_token == output_token && parent_idx.is_some() {
            if current_amount > best_output {
                best_output = current_amount;
                best_path_end = parent_idx;
            }
            continue; // don't explore past destination
        }

        // The request processor will return a no path error if the route is too long
        // This limits the search space
        if hop_count >= MAX_HOPS {
            continue;
        }

        // Try all tokens reachable in one trade
        for edge in snapshot.neighbors(&current_token) {
            // Skip stale or unhealthy books
            if !snapshot.is_usable(&edge.pair) {
                state
                    .metrics
                    .stale_or_unhealthy_skipped
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }

            // Get the orderbook from snapshot using pair key
            let Some(book) = snapshot.get_orderbook(&edge.pair) else {
                continue;
            };

            let result = match_order(book, edge.side, current_amount);

            // Skip if no liquidity on this path
            if result.output_produced == 0 {
                continue;
            }

            // Add node to arena and enqueue
            let node_idx = arena.len();
            arena.push(PathNode {
                swap: Swap {
                    input_token: current_token,
                    output_token: edge.target,
                    direction: edge.side,
                    input_amount: result.input_consumed,
                    expected_output_amount: result.output_produced,
                },
                parent: parent_idx,
            });

            queue.push_back((
                edge.target,
                result.output_produced,
                hop_count + 1,
                Some(node_idx),
            ));
        }
    }

    best_path_end.map(|idx| reconstruct_path(&arena, idx))
}
