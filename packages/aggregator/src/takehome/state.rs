use std::fmt;
use std::sync::Arc;

use aggregator_utils::orderbook::OrderbookState;
use aggregator_utils::types::{Address, Side};
use scc::HashMap as SccHashMap;

/// Represents one direction you can trade through an orderbook.
/// Each orderbook creates two edges: one for buying, one for selling.
#[derive(Debug, Clone)]
pub struct GraphEdge {
    pub target: Address,
    pub pair: (Address, Address), // key into orderbooks hashmap
    pub side: Side,               // Ask = buying target, Bid = selling for target
}

/// Shared state using scc::HashMap for lock-free concurrent access.
///
/// Previously used RwLock<HashMap>, but profiling showed 150-300μs writer
/// blocking when readers held the lock. With 100 writes/sec and 10 reads/sec,
/// this caused measurable writer starvation.
///
/// scc::HashMap eliminates this via bucket-level locking. Routing takes a
/// snapshot (~10-50μs) to ensure consistent BFS traversal.
#[derive(Default)]
pub struct AggregatorState {
    /// Lock-free concurrent hashmap for orderbooks.
    /// Key: (base_token, quote_token) pair
    pub orderbooks: SccHashMap<(Address, Address), OrderbookState>,

    /// Lock-free concurrent hashmap for graph edges.
    /// Key: source token, Value: edges to neighboring tokens
    pub graph_edges: SccHashMap<Address, Vec<GraphEdge>>,
}

impl AggregatorState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Upsert orderbook. Takes &self (not &mut self) since scc provides interior mutability.
    pub fn upsert_orderbook(&self, book: OrderbookState) {
        let pair = (book.base_token, book.quote_token);

        // Check if this is a new pair (need to add graph edges)
        let is_new = !self.orderbooks.contains(&pair);

        // Upsert the orderbook
        let _ = self.orderbooks.upsert(pair, book);

        // Add graph edges for new pairs only
        if is_new {
            self.add_graph_edges(pair);
        }
    }

    fn add_graph_edges(&self, pair: (Address, Address)) {
        let (base, quote) = pair;

        // Edge: quote -> base (buying base with quote, Ask side)
        self.graph_edges
            .entry(quote)
            .or_default()
            .get_mut()
            .push(GraphEdge {
                target: base,
                pair,
                side: Side::Ask,
            });

        // Edge: base -> quote (selling base for quote, Bid side)
        self.graph_edges
            .entry(base)
            .or_default()
            .get_mut()
            .push(GraphEdge {
                target: quote,
                pair,
                side: Side::Bid,
            });
    }

    /// Check if token exists in graph
    pub fn contains_token(&self, token: Address) -> bool {
        self.graph_edges.contains(&token)
    }

    /// Get neighbors for routing (returns clone since we can't hold reference across await)
    pub fn neighbors(&self, token: Address) -> Vec<GraphEdge> {
        self.graph_edges
            .read(&token, |_, edges| edges.clone())
            .unwrap_or_default()
    }

    /// Get orderbook by pair
    pub fn get_orderbook(&self, pair: &(Address, Address)) -> Option<OrderbookState> {
        self.orderbooks.read(pair, |_, book| book.clone())
    }

    /// Get count of orderbooks (for tests)
    pub fn orderbook_count(&self) -> usize {
        self.orderbooks.len()
    }
}

// Manual Debug implementation since scc::HashMap doesn't implement Debug
impl fmt::Debug for AggregatorState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AggregatorState")
            .field("orderbooks_count", &self.orderbooks.len())
            .field("graph_edges_count", &self.graph_edges.len())
            .finish()
    }
}

// SharedState is now just Arc<AggregatorState> - no RwLock needed
pub type SharedState = Arc<AggregatorState>;

pub fn create_shared_state() -> SharedState {
    Arc::new(AggregatorState::new())
}
