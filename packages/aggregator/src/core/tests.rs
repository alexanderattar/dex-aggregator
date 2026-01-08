use aggregator_utils::{
    orderbook::{OrderbookLevel, OrderbookState},
    types::{Address, Side, SwapRequest},
};

use crate::core::{
    event_processor::DexEventProcessor,
    matching::match_order,
    request_processor::DexRequestProcessor,
    routing::find_best_route,
    state::{create_shared_state, AggregatorState},
};
use crate::traits::{EventProcessor, RequestProcessor};

fn simple_book(
    base: Address,
    quote: Address,
    bid_px: u64,
    ask_px: u64,
    size: u64,
) -> OrderbookState {
    let mut ob = OrderbookState::new(base, quote);
    ob.insert_bid(OrderbookLevel {
        px: bid_px,
        sz: size,
    });
    ob.insert_ask(OrderbookLevel {
        px: ask_px,
        sz: size,
    });
    ob
}

#[test]
fn edges_created_both_directions() {
    let base = Address::new_random();
    let quote = Address::new_random();
    let ob = simple_book(base, quote, 10, 12, 100);

    let state = AggregatorState::new();
    state.upsert_orderbook(ob);

    let neighbors_base = state.neighbors(base);
    let neighbors_quote = state.neighbors(quote);

    assert_eq!(neighbors_base.len(), 1);
    assert_eq!(neighbors_quote.len(), 1);
    assert_eq!(neighbors_base[0].target, quote);
    assert_eq!(neighbors_base[0].side, Side::Bid);
    assert_eq!(neighbors_quote[0].target, base);
    assert_eq!(neighbors_quote[0].side, Side::Ask);
}

#[test]
fn route_finds_direct_path() {
    let a = Address::new_random();
    let b = Address::new_random();
    let ob = simple_book(a, b, 20, 22, 100);

    let state = AggregatorState::new();
    state.upsert_orderbook(ob);

    let route = find_best_route(&state, a, b, 50).expect("route");
    assert_eq!(route.len(), 1);
    assert!(route[0].expected_output_amount > 0);
}

#[test]
fn multi_hop_route() {
    let a = Address::new_random();
    let b = Address::new_random();
    let c = Address::new_random();

    let state = AggregatorState::new();
    state.upsert_orderbook(simple_book(a, c, 100, 102, 1000));
    state.upsert_orderbook(simple_book(c, b, 100, 102, 1000));

    let route = find_best_route(&state, a, b, 100).expect("route");
    assert_eq!(route.len(), 2);
    assert_eq!(route[0].input_token, a);
    assert_eq!(route[0].output_token, c);
    assert_eq!(route[1].input_token, c);
    assert_eq!(route[1].output_token, b);
}

#[test]
fn matching_handles_edge_cases() {
    let a = Address::new_random();
    let b = Address::new_random();
    let mut ob = OrderbookState::new(a, b);
    ob.insert_ask(OrderbookLevel {
        px: 0,
        sz: u64::MAX,
    });
    ob.insert_bid(OrderbookLevel {
        px: u64::MAX,
        sz: u64::MAX,
    });

    // Zero price ask should produce nothing
    let ask_result = match_order(&ob, Side::Ask, 10);
    assert_eq!(ask_result.output_produced, 0);

    // Large bid should saturate, not overflow
    let bid_result = match_order(&ob, Side::Bid, 2);
    assert!(bid_result.output_produced > 0);
}

#[test]
fn upsert_updates_existing_orderbook() {
    let a = Address::new_random();
    let b = Address::new_random();

    let state = AggregatorState::new();
    state.upsert_orderbook(simple_book(a, b, 10, 12, 100));
    assert_eq!(state.orderbook_count(), 1);

    // Same pair should update, not insert
    state.upsert_orderbook(simple_book(a, b, 11, 13, 200));
    assert_eq!(state.orderbook_count(), 1);
}

#[tokio::test]
async fn request_processor_returns_route() {
    let shared = create_shared_state();
    let a = Address::new_random();
    let b = Address::new_random();

    {
        let processor = DexEventProcessor::new(shared.clone());
        processor
            .process_orderbook(simple_book(a, b, 10, 12, 100))
            .unwrap();
    }

    let processor = DexRequestProcessor::new(shared);
    let req = SwapRequest {
        input_token: a,
        output_token: b,
        input_amount: 50,
        min_output_amount: 0,
    };

    let resp = processor.process_request(req).await.unwrap();
    assert!(matches!(
        resp,
        aggregator_utils::types::SwapResponse::Success(_)
    ));
}

#[test]
fn no_path_returns_none() {
    let a = Address::new_random();
    let b = Address::new_random();
    let c = Address::new_random();

    let state = AggregatorState::new();
    // a-b connected, but c is isolated
    state.upsert_orderbook(simple_book(a, b, 10, 12, 100));

    assert!(find_best_route(&state, a, c, 100).is_none());
}

#[tokio::test]
async fn rejects_same_token() {
    use aggregator_utils::types::SwapResponse;

    let shared = create_shared_state();
    let a = Address::new_random();

    let processor = DexRequestProcessor::new(shared);
    let req = SwapRequest {
        input_token: a,
        output_token: a,
        input_amount: 100,
        min_output_amount: 0,
    };

    let resp = processor.process_request(req).await.unwrap();
    assert!(matches!(resp, SwapResponse::Failure(msg) if msg == "same token"));
}

#[tokio::test]
async fn rejects_unknown_token() {
    use aggregator_utils::types::SwapResponse;

    let shared = create_shared_state();
    let a = Address::new_random();
    let b = Address::new_random();

    // No orderbooks added, tokens are unknown
    let processor = DexRequestProcessor::new(shared);
    let req = SwapRequest {
        input_token: a,
        output_token: b,
        input_amount: 100,
        min_output_amount: 0,
    };

    let resp = processor.process_request(req).await.unwrap();
    // Error message sanitized to avoid leaking token existence info
    assert!(matches!(resp, SwapResponse::Failure(msg) if msg == "invalid request"));
}

#[tokio::test]
async fn rejects_slippage() {
    use aggregator_utils::types::SwapResponse;

    let shared = create_shared_state();
    let a = Address::new_random();
    let b = Address::new_random();

    {
        let processor = DexEventProcessor::new(shared.clone());
        processor
            .process_orderbook(simple_book(a, b, 10, 12, 100))
            .unwrap();
    }

    let processor = DexRequestProcessor::new(shared);
    let req = SwapRequest {
        input_token: a,
        output_token: b,
        input_amount: 50,
        min_output_amount: u64::MAX, // Impossible to satisfy
    };

    let resp = processor.process_request(req).await.unwrap();
    assert!(matches!(resp, SwapResponse::Failure(msg) if msg.starts_with("slippage")));
}

#[test]
fn picks_best_route_among_alternatives() {
    let a = Address::new_random();
    let b = Address::new_random();
    let c = Address::new_random();

    let state = AggregatorState::new();

    // Direct route a->b with bad rate (price 20)
    let mut direct = OrderbookState::new(a, b);
    direct.insert_bid(OrderbookLevel { px: 20, sz: 1000 });
    state.upsert_orderbook(direct);

    // Indirect route a->c->b with better effective rate
    let mut ac = OrderbookState::new(a, c);
    ac.insert_bid(OrderbookLevel { px: 100, sz: 1000 }); // a sells for 100 c each
    state.upsert_orderbook(ac);

    let mut cb = OrderbookState::new(c, b);
    cb.insert_bid(OrderbookLevel { px: 50, sz: 100000 }); // c sells for 50 b each
    state.upsert_orderbook(cb);

    // Swap 10 a -> b
    // Direct: 10 * 20 = 200 b
    // Indirect: 10 * 100 = 1000 c, then 1000 * 50 = 50000 b
    let route = find_best_route(&state, a, b, 10).expect("route");
    let output = route.last().unwrap().expected_output_amount;

    // Should pick the indirect route with much higher output
    assert!(
        output > 200,
        "expected indirect route, got output {}",
        output
    );
}

#[test]
fn partial_fill_limited_liquidity() {
    let a = Address::new_random();
    let b = Address::new_random();

    // Only 10 units available at price 100
    let mut ob = OrderbookState::new(a, b);
    ob.insert_bid(OrderbookLevel { px: 100, sz: 10 });

    let result = match_order(&ob, Side::Bid, 50); // Try to sell 50

    // Should only fill 10 (all available liquidity)
    assert_eq!(result.input_consumed, 10);
    assert_eq!(result.output_produced, 1000); // 10 * 100
}

// ============================================================================
// Health and Staleness Tests
// ============================================================================

#[test]
fn validate_orderbook_rejects_crossed_spread() {
    let a = Address::new_random();
    let b = Address::new_random();

    // Crossed book: bid (15) >= ask (10)
    let mut crossed = OrderbookState::new(a, b);
    crossed.insert_bid(OrderbookLevel { px: 15, sz: 100 });
    crossed.insert_ask(OrderbookLevel { px: 10, sz: 100 });

    assert!(!AggregatorState::validate_orderbook(&crossed));
}

#[test]
fn validate_orderbook_rejects_empty_book() {
    let a = Address::new_random();
    let b = Address::new_random();

    // Empty book (no levels)
    let empty = OrderbookState::new(a, b);
    assert!(!AggregatorState::validate_orderbook(&empty));
}

#[test]
fn validate_orderbook_accepts_one_sided_book() {
    let a = Address::new_random();
    let b = Address::new_random();

    // Only bids (acceptable during market stress)
    let mut bid_only = OrderbookState::new(a, b);
    bid_only.insert_bid(OrderbookLevel { px: 10, sz: 100 });
    assert!(AggregatorState::validate_orderbook(&bid_only));

    // Only asks
    let mut ask_only = OrderbookState::new(a, b);
    ask_only.insert_ask(OrderbookLevel { px: 20, sz: 100 });
    assert!(AggregatorState::validate_orderbook(&ask_only));
}

#[test]
fn validate_orderbook_accepts_valid_spread() {
    let a = Address::new_random();
    let b = Address::new_random();

    // Valid book: bid (10) < ask (12)
    let valid = simple_book(a, b, 10, 12, 100);
    assert!(AggregatorState::validate_orderbook(&valid));
}

#[test]
fn is_pair_usable_checks_health_and_staleness() {
    use std::time::Duration;

    let state = AggregatorState::new();
    let a = Address::new_random();
    let b = Address::new_random();
    let pair = (a, b);

    // Insert valid book
    state.upsert_orderbook(simple_book(a, b, 10, 12, 100));

    // Should be usable immediately
    assert!(state.is_pair_usable(&pair, Duration::from_secs(30)));

    // Non-existent pair should not be usable
    let unknown = (Address::new_random(), Address::new_random());
    assert!(!state.is_pair_usable(&unknown, Duration::from_secs(30)));
}

#[test]
fn event_processor_rejects_crossed_book() {
    let shared = create_shared_state();
    let processor = DexEventProcessor::new(shared.clone());

    let a = Address::new_random();
    let b = Address::new_random();

    // Try to insert crossed book
    let mut crossed = OrderbookState::new(a, b);
    crossed.insert_bid(OrderbookLevel { px: 15, sz: 100 });
    crossed.insert_ask(OrderbookLevel { px: 10, sz: 100 });

    let result = processor.process_orderbook(crossed);
    assert!(result.is_err());

    // Metrics should track invalid event
    assert_eq!(
        shared
            .metrics
            .events_invalid
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

// ============================================================================
// Circuit Breaker Tests
// ============================================================================

#[test]
fn circuit_breaker_starts_closed() {
    use crate::core::state::CircuitBreaker;

    let cb = CircuitBreaker::default();
    assert_eq!(cb.current_state(), "closed");
    assert!(cb.allow_request());
}

#[test]
fn circuit_breaker_opens_after_threshold() {
    use crate::core::state::CircuitBreaker;
    use std::time::Duration;

    let cb = CircuitBreaker::new(3, Duration::from_secs(10));

    // First 2 failures keep circuit closed
    cb.record_failure();
    cb.record_failure();
    assert_eq!(cb.current_state(), "closed");
    assert!(cb.allow_request());

    // Third failure opens circuit
    cb.record_failure();
    assert_eq!(cb.current_state(), "open");
    assert!(!cb.allow_request());
}

#[test]
fn circuit_breaker_success_resets_failures() {
    use crate::core::state::CircuitBreaker;
    use std::time::Duration;

    let cb = CircuitBreaker::new(3, Duration::from_secs(10));

    cb.record_failure();
    cb.record_failure();
    assert_eq!(cb.failure_count(), 2);

    // Success resets counter
    cb.record_success();
    assert_eq!(cb.failure_count(), 0);

    // Need 3 more failures to open
    cb.record_failure();
    cb.record_failure();
    assert_eq!(cb.current_state(), "closed");
}

#[tokio::test]
async fn circuit_breaker_rejects_when_open() {
    use aggregator_utils::types::SwapResponse;

    let shared = create_shared_state();
    let a = Address::new_random();
    let b = Address::new_random();

    // Force circuit breaker open by recording failures
    for _ in 0..5 {
        shared.circuit_breaker.record_failure();
    }
    assert_eq!(shared.circuit_breaker.current_state(), "open");

    let processor = DexRequestProcessor::new(shared);
    let req = SwapRequest {
        input_token: a,
        output_token: b,
        input_amount: 100,
        min_output_amount: 0,
    };

    let resp = processor.process_request(req).await.unwrap();
    assert!(matches!(resp, SwapResponse::Failure(msg) if msg == "service degraded"));
}

// ============================================================================
// Metrics Tests
// ============================================================================

#[tokio::test]
async fn metrics_track_requests() {
    let shared = create_shared_state();
    let a = Address::new_random();
    let b = Address::new_random();

    {
        let processor = DexEventProcessor::new(shared.clone());
        processor
            .process_orderbook(simple_book(a, b, 10, 12, 100))
            .unwrap();
    }

    let processor = DexRequestProcessor::new(shared.clone());

    // Successful request
    let req = SwapRequest {
        input_token: a,
        output_token: b,
        input_amount: 50,
        min_output_amount: 0,
    };
    let _ = processor.process_request(req).await;

    assert_eq!(
        shared
            .metrics
            .requests_total
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    // Failed request (unknown token)
    let req2 = SwapRequest {
        input_token: Address::new_random(),
        output_token: Address::new_random(),
        input_amount: 50,
        min_output_amount: 0,
    };
    let _ = processor.process_request(req2).await;

    assert_eq!(
        shared
            .metrics
            .requests_total
            .load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    assert_eq!(
        shared
            .metrics
            .requests_failed
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}
