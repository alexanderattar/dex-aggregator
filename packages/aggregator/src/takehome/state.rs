use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use aggregator_utils::orderbook::OrderbookState;
use aggregator_utils::types::{Address, Side};

// Represents one direction you can trade through an orderbook.
// Each orderbook creates two edges: one for buying, one for selling.
#[derive(Debug, Clone)]
pub struct GraphEdge {
    pub target: Address,
    pub orderbook_idx: usize, // index into AggregatorState::orderbooks
    pub side: Side,           // Ask = buying target, Bid = selling for target
}

// Token graph for route discovery. HashMap gives O(1) neighbor lookup
// which matters since routing calls neighbors() at every hop.
#[derive(Debug, Clone, Default)]
pub struct TokenGraph {
    adjacency: HashMap<Address, Vec<GraphEdge>>,
}

impl TokenGraph {
    // Check if the token is in the graph
    pub fn contains(&self, token: Address) -> bool {
        self.adjacency.contains_key(&token)
    }

    // Get the neighbors of the token
    pub fn neighbors(&self, token: Address) -> &[GraphEdge] {
        self.adjacency
            .get(&token)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    // Each orderbook creates bidirectional edges between its token pair
    fn add_orderbook(&mut self, idx: usize, base: Address, quote: Address) {
        // quote -> base: buy base with quote (asks)
        self.adjacency.entry(quote).or_default().push(GraphEdge {
            target: base,
            orderbook_idx: idx,
            side: Side::Ask,
        });
        // base -> quote: sell base for quote (bids)
        self.adjacency.entry(base).or_default().push(GraphEdge {
            target: quote,
            orderbook_idx: idx,
            side: Side::Bid,
        });
    }
}

// Core state with two indexes optimized for different access patterns:
// - orderbooks Vec: O(1) by index during route simulation
// - orderbook_index HashMap: O(1) by token pair for upsert dedup
// - graph: O(1) neighbor lookup for route discovery
#[derive(Debug, Default)]
pub struct AggregatorState {
    pub orderbooks: Vec<OrderbookState>,
    orderbook_index: HashMap<(Address, Address), usize>,
    pub graph: TokenGraph,
}

impl AggregatorState {
    pub fn new() -> Self {
        Self {
            orderbooks: Vec::new(),
            orderbook_index: HashMap::new(),
            graph: TokenGraph::default(),
        }
    }

    // Price updates just replace in-place. New pairs also add graph edges.
    // Graph only grows when we see a new token pair (rare after warmup).
    pub fn upsert_orderbook(&mut self, book: OrderbookState) {
        let key = (book.base_token, book.quote_token);

        if let Some(&idx) = self.orderbook_index.get(&key) {
            // existing pair: update prices, graph unchanged
            self.orderbooks[idx] = book;
        } else {
            // new pair: add to graph
            let idx = self.orderbooks.len();
            self.graph
                .add_orderbook(idx, book.base_token, book.quote_token);
            self.orderbook_index.insert(key, idx);
            self.orderbooks.push(book);
        }
    }
}

// RwLock protects shared state for concurrent readers and writers
pub type SharedState = Arc<RwLock<AggregatorState>>;

// Helper function to create a shared state
pub fn create_shared_state() -> SharedState {
    Arc::new(RwLock::new(AggregatorState::new()))
}
