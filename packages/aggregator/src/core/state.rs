use std::{
    fmt,
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
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
    pub pair: (Address, Address), // key into orderbooks hashmap
    pub side: Side,               // Ask = buying target, Bid = selling for target
}

/// Metadata for an orderbook, tracking health and freshness.
#[derive(Debug, Clone)]
pub struct BookMeta {
    /// Last time this orderbook was updated.
    pub updated_at: Instant,
    /// Whether the orderbook passes validation (no crossed spreads, has liquidity).
    pub healthy: bool,
}

/// Atomic counters for aggregator metrics.
///
/// All counters use relaxed ordering since they are for monitoring only
/// and do not require strict synchronization.
#[derive(Debug, Default)]
pub struct Metrics {
    /// Total orderbook events received.
    pub events_total: AtomicU64,
    /// Orderbook events rejected due to validation failure.
    pub events_invalid: AtomicU64,
    /// Total swap requests received.
    pub requests_total: AtomicU64,
    /// Swap requests that failed (no route, unknown tokens, etc.).
    pub requests_failed: AtomicU64,
    /// Swap requests rejected by rate limiter.
    pub requests_rate_limited: AtomicU64,
    /// Number of routing snapshots created.
    pub snapshots_taken: AtomicU64,
    /// Orderbooks skipped during routing due to staleness or health issues.
    pub stale_or_unhealthy_skipped: AtomicU64,
}

/// Circuit breaker states
const CB_CLOSED: u8 = 0; // Normal operation, requests flow through
const CB_OPEN: u8 = 1; // Failures exceeded threshold, reject all requests
const CB_HALF_OPEN: u8 = 2; // Testing if service recovered, allow limited requests

/// Circuit breaker for graceful degradation under failure conditions.
/// Opens when consecutive failures exceed threshold, preventing cascade failures.
/// Automatically attempts recovery after cooldown period.
pub struct CircuitBreaker {
    state: AtomicU8,
    consecutive_failures: AtomicU64,
    /// Mutex instead of RwLock to prevent TOCTOU race conditions during state transitions.
    /// State changes require atomic read-check-write which RwLock cannot guarantee.
    last_state_change: Mutex<Instant>,
    /// Number of consecutive failures before opening circuit
    failure_threshold: u64,
    /// Time to wait before attempting recovery (half-open state)
    recovery_timeout: Duration,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(CB_CLOSED),
            consecutive_failures: AtomicU64::new(0),
            last_state_change: Mutex::new(Instant::now()),
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(10),
        }
    }
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u64, recovery_timeout: Duration) -> Self {
        Self {
            state: AtomicU8::new(CB_CLOSED),
            consecutive_failures: AtomicU64::new(0),
            last_state_change: Mutex::new(Instant::now()),
            failure_threshold,
            recovery_timeout,
        }
    }

    /// Check if request should be allowed through.
    /// Returns true if circuit is closed or half-open (testing recovery).
    pub fn allow_request(&self) -> bool {
        let state = self.state.load(Ordering::Acquire);

        match state {
            CB_CLOSED => true,
            CB_OPEN => {
                // Hold mutex during entire check-and-transition to prevent TOCTOU race
                let mut last_change = self.last_state_change.lock().unwrap();
                if last_change.elapsed() >= self.recovery_timeout {
                    // Transition to half-open to test recovery
                    if self
                        .state
                        .compare_exchange(
                            CB_OPEN,
                            CB_HALF_OPEN,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        *last_change = Instant::now();
                        return true;
                    }
                }
                false
            }
            CB_HALF_OPEN => true, // Allow test request
            _ => false,
        }
    }

    /// Record a successful request. Resets failure count and closes circuit if half-open.
    pub fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Release);

        let state = self.state.load(Ordering::Acquire);
        if state == CB_HALF_OPEN {
            // Recovery confirmed, close circuit
            self.state.store(CB_CLOSED, Ordering::Release);
            *self.last_state_change.lock().unwrap() = Instant::now();
        }
    }

    /// Record a failed request. Opens circuit if failures exceed threshold.
    pub fn record_failure(&self) {
        let failures = self.consecutive_failures.fetch_add(1, Ordering::AcqRel) + 1;
        let state = self.state.load(Ordering::Acquire);

        match state {
            CB_CLOSED => {
                if failures >= self.failure_threshold {
                    // Too many failures, open circuit
                    if self
                        .state
                        .compare_exchange(CB_CLOSED, CB_OPEN, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        *self.last_state_change.lock().unwrap() = Instant::now();
                    }
                }
            }
            CB_HALF_OPEN => {
                // Recovery test failed, back to open
                self.state.store(CB_OPEN, Ordering::Release);
                *self.last_state_change.lock().unwrap() = Instant::now();
            }
            _ => {}
        }
    }

    /// Get current state for monitoring/debugging
    pub fn current_state(&self) -> &'static str {
        match self.state.load(Ordering::Acquire) {
            CB_CLOSED => "closed",
            CB_OPEN => "open",
            CB_HALF_OPEN => "half-open",
            _ => "unknown",
        }
    }

    /// Get consecutive failure count
    pub fn failure_count(&self) -> u64 {
        self.consecutive_failures.load(Ordering::Acquire)
    }
}

impl fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("state", &self.current_state())
            .field("failures", &self.failure_count())
            .finish()
    }
}

/// Token-bucket rate limiter for request throttling.
/// Prevents abuse and provides backpressure under high load.
pub struct RateLimiter {
    /// Available tokens (scaled by 1000 for sub-token precision)
    tokens: AtomicU64,
    /// Last time tokens were refilled
    last_refill: Mutex<Instant>,
    /// Tokens added per second
    tokens_per_second: u64,
    /// Maximum token capacity (burst size)
    max_tokens: u64,
}

impl RateLimiter {
    /// Create a new rate limiter.
    /// `tokens_per_second`: steady-state request rate
    /// `max_tokens`: burst capacity
    pub fn new(tokens_per_second: u64, max_tokens: u64) -> Self {
        Self {
            tokens: AtomicU64::new(max_tokens * 1000), // scaled
            last_refill: Mutex::new(Instant::now()),
            tokens_per_second,
            max_tokens,
        }
    }

    /// Attempt to acquire a token. Returns true if successful.
    pub fn try_acquire(&self) -> bool {
        self.refill();

        // Try to consume one token (1000 scaled units)
        let mut current = self.tokens.load(Ordering::Acquire);
        loop {
            if current < 1000 {
                return false; // Not enough tokens
            }

            match self.tokens.compare_exchange_weak(
                current,
                current - 1000,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(new_val) => current = new_val,
            }
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&self) {
        let mut last = self.last_refill.lock().unwrap();
        let elapsed = last.elapsed();

        // Only refill if at least 1ms has passed (avoid excessive lock contention)
        if elapsed.as_millis() < 1 {
            return;
        }

        let tokens_to_add = (elapsed.as_millis() as u64 * self.tokens_per_second) / 1000 * 1000;
        if tokens_to_add > 0 {
            let max_scaled = self.max_tokens * 1000;
            let current = self.tokens.load(Ordering::Acquire);
            let new_tokens = (current + tokens_to_add).min(max_scaled);
            self.tokens.store(new_tokens, Ordering::Release);
            *last = Instant::now();
        }
    }

    /// Get current available tokens (for monitoring)
    pub fn available_tokens(&self) -> u64 {
        self.tokens.load(Ordering::Acquire) / 1000
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        // Default: 1000 req/sec with burst of 100
        Self::new(1000, 100)
    }
}

impl fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimiter")
            .field("available_tokens", &self.available_tokens())
            .field("tokens_per_second", &self.tokens_per_second)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

/// Shared state using scc::HashMap for lock-free concurrent access with per-book health.
#[derive(Default)]
pub struct AggregatorState {
    /// Lock-free concurrent hashmap for orderbooks.
    /// Key: (base_token, quote_token) pair
    pub orderbooks: SccHashMap<(Address, Address), OrderbookState>,

    /// Lock-free concurrent hashmap for graph edges.
    /// Key: source token, Value: edges to neighboring tokens
    pub graph_edges: SccHashMap<Address, Vec<GraphEdge>>,

    /// Metadata per orderbook (staleness, health).
    pub orderbook_meta: SccHashMap<(Address, Address), BookMeta>,

    /// Simple in-process counters.
    pub metrics: Metrics,

    /// Circuit breaker for graceful degradation.
    pub circuit_breaker: CircuitBreaker,

    /// Rate limiter for request throttling.
    pub rate_limiter: RateLimiter,
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

        let healthy = Self::validate_orderbook(&book);

        // Upsert the orderbook and metadata
        let _ = self.orderbooks.upsert(pair, book);
        let _ = self.orderbook_meta.upsert(
            pair,
            BookMeta {
                updated_at: Instant::now(),
                healthy,
            },
        );

        // Add graph edges for new pairs only
        if is_new {
            self.add_graph_edges(pair);
        }
    }

    /// Basic validation: non-empty sides and no crossed book (best bid < best ask).
    pub fn validate_orderbook(book: &OrderbookState) -> bool {
        let mut best_bid: Option<u64> = None;
        for b in book.bids() {
            if b.sz == 0 {
                continue;
            }
            best_bid = Some(b.px);
            break;
        }

        let mut best_ask: Option<u64> = None;
        for a in book.asks() {
            if a.sz == 0 {
                continue;
            }
            best_ask = Some(a.px);
            break;
        }

        match (best_bid, best_ask) {
            (Some(bid), Some(ask)) => bid < ask, // both sides present, not crossed
            (Some(_), None) | (None, Some(_)) => true, // one-sided book is acceptable
            _ => false,                          // no usable levels
        }
    }

    /// Check meta for health and staleness.
    pub fn is_pair_usable(&self, pair: &(Address, Address), max_age: Duration) -> bool {
        self.orderbook_meta
            .read(pair, |_, meta| {
                meta.healthy && meta.updated_at.elapsed() <= max_age
            })
            .unwrap_or(false)
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

    /// Get metadata by pair
    pub fn get_meta(&self, pair: &(Address, Address)) -> Option<BookMeta> {
        self.orderbook_meta.read(pair, |_, meta| meta.clone())
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
            .field("circuit_breaker", &self.circuit_breaker)
            .field("rate_limiter", &self.rate_limiter)
            .finish()
    }
}

// SharedState is now just Arc<AggregatorState> - no RwLock needed
pub type SharedState = Arc<AggregatorState>;

pub fn create_shared_state() -> SharedState {
    Arc::new(AggregatorState::new())
}
