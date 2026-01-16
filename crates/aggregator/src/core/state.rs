use std::{
    fmt,
    sync::{atomic::AtomicU64, Arc},
    time::{Duration, Instant},
};

use aggregator_utils::orderbook::OrderbookState;
use aggregator_utils::types::{Address, Side};
use scc::HashMap as SccHashMap;

/// Represents one direction you can trade through an orderbook.
/// Each orderbook creates two edges: one for buying, one for selling.
#[derive(Debug, Clone)]
pub struct GraphEdge {
    pub target: Address,
    pub pair: (Address, Address),
    pub side: Side,
}

/// Tracks orderbook freshness and validity for routing decisions.
#[derive(Debug, Clone)]
pub struct OrderbookHealth {
    pub last_updated: Instant,
    pub has_valid_spread: bool,
}

#[derive(Debug, Default)]
pub struct Metrics {
    pub events_total: AtomicU64,
    pub events_invalid: AtomicU64,
    pub requests_total: AtomicU64,
    pub requests_failed: AtomicU64,
    pub snapshots_taken: AtomicU64,
    pub stale_or_unhealthy_skipped: AtomicU64,
}

/// Shared state using scc::HashMap for lock-free concurrent access.
#[derive(Default)]
pub struct AggregatorState {
    pub orderbooks: SccHashMap<(Address, Address), OrderbookState>,
    pub graph_edges: SccHashMap<Address, Vec<GraphEdge>>,
    pub orderbook_health: SccHashMap<(Address, Address), OrderbookHealth>,
    pub metrics: Metrics,
}

impl AggregatorState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_orderbook(&self, book: OrderbookState) {
        let pair = (book.base_token, book.quote_token);
        let is_new = !self.orderbooks.contains(&pair);
        let has_valid_spread = Self::validate_orderbook(&book);

        let _ = self.orderbooks.upsert(pair, book);
        let _ = self.orderbook_health.upsert(
            pair,
            OrderbookHealth {
                last_updated: Instant::now(),
                has_valid_spread,
            },
        );

        if is_new {
            self.add_graph_edges(pair);
        }
    }

    /// Validates orderbook has liquidity and no crossed spread.
    pub fn validate_orderbook(book: &OrderbookState) -> bool {
        let best_bid = book.bids().iter().find(|b| b.sz > 0).map(|b| b.px);
        let best_ask = book.asks().iter().find(|a| a.sz > 0).map(|a| a.px);

        match (best_bid, best_ask) {
            (Some(bid), Some(ask)) => bid < ask,
            (Some(_), None) | (None, Some(_)) => true,
            _ => false,
        }
    }

    pub fn is_pair_usable(&self, pair: &(Address, Address), max_age: Duration) -> bool {
        self.orderbook_health
            .read(pair, |_, health| {
                health.has_valid_spread && health.last_updated.elapsed() <= max_age
            })
            .unwrap_or(false)
    }

    fn add_graph_edges(&self, pair: (Address, Address)) {
        let (base, quote) = pair;

        self.graph_edges
            .entry(quote)
            .or_default()
            .get_mut()
            .push(GraphEdge {
                target: base,
                pair,
                side: Side::Ask,
            });

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

    pub fn contains_token(&self, token: Address) -> bool {
        self.graph_edges.contains(&token)
    }

    pub fn neighbors(&self, token: Address) -> Vec<GraphEdge> {
        self.graph_edges
            .read(&token, |_, edges| edges.clone())
            .unwrap_or_default()
    }

    pub fn get_orderbook(&self, pair: &(Address, Address)) -> Option<OrderbookState> {
        self.orderbooks.read(pair, |_, book| book.clone())
    }

    pub fn get_health(&self, pair: &(Address, Address)) -> Option<OrderbookHealth> {
        self.orderbook_health.read(pair, |_, health| health.clone())
    }

    pub fn orderbook_count(&self) -> usize {
        self.orderbooks.len()
    }
}

impl fmt::Debug for AggregatorState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AggregatorState")
            .field("orderbooks_count", &self.orderbooks.len())
            .field("graph_edges_count", &self.graph_edges.len())
            .finish()
    }
}

pub type SharedState = Arc<AggregatorState>;

pub fn create_shared_state() -> SharedState {
    Arc::new(AggregatorState::new())
}
