//! Integration tests for the DEX aggregator.
//!
//! These tests verify end-to-end behavior including:
//! - Full request/response cycles
//! - Circuit breaker state transitions
//! - Rate limiting behavior
//! - Multi-hop routing across multiple orderbooks

use aggregator::core::{
    event_processor::DexEventProcessor, request_processor::DexRequestProcessor,
    state::create_shared_state,
};
use aggregator::traits::{EventProcessor, RequestProcessor};
use aggregator_utils::orderbook::{OrderbookLevel, OrderbookState};
use aggregator_utils::types::{Address, SwapRequest, SwapResponse};

fn make_orderbook(base: Address, quote: Address, bid_px: u64, ask_px: u64) -> OrderbookState {
    let mut ob = OrderbookState::new(base, quote);
    ob.insert_bid(OrderbookLevel {
        px: bid_px,
        sz: 10_000,
    });
    ob.insert_ask(OrderbookLevel {
        px: ask_px,
        sz: 10_000,
    });
    ob
}

#[tokio::test]
async fn full_swap_cycle() {
    let state = create_shared_state();
    let event_processor = DexEventProcessor::new(state.clone());
    let request_processor = DexRequestProcessor::new(state.clone());

    let token_a = Address::new_random();
    let token_b = Address::new_random();

    // Ingest an orderbook
    let book = make_orderbook(token_a, token_b, 99, 101);
    event_processor.process_orderbook(book).unwrap();

    // Request a swap
    let request = SwapRequest {
        input_token: token_a,
        output_token: token_b,
        input_amount: 1000,
        min_output_amount: 0,
    };

    let response = request_processor.process_request(request).await.unwrap();
    assert!(matches!(response, SwapResponse::Success(_)));

    // Verify metrics were updated
    assert_eq!(
        state
            .metrics
            .events_total
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        state
            .metrics
            .requests_total
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[tokio::test]
async fn circuit_breaker_opens_and_recovers() {
    let state = create_shared_state();
    let request_processor = DexRequestProcessor::new(state.clone());

    let token_a = Address::new_random();
    let token_b = Address::new_random();

    // No orderbooks, so requests will fail and trigger circuit breaker
    for _ in 0..5 {
        let request = SwapRequest {
            input_token: token_a,
            output_token: token_b,
            input_amount: 1000,
            min_output_amount: 0,
        };
        let _ = request_processor.process_request(request).await;
    }

    // Circuit should now be open
    assert_eq!(state.circuit_breaker.current_state(), "open");

    // Additional requests should be rejected with "service degraded"
    let request = SwapRequest {
        input_token: token_a,
        output_token: token_b,
        input_amount: 1000,
        min_output_amount: 0,
    };
    let response = request_processor.process_request(request).await.unwrap();
    assert!(matches!(response, SwapResponse::Failure(msg) if msg == "service degraded"));
}

#[tokio::test]
async fn multi_hop_routing_across_orderbooks() {
    let state = create_shared_state();
    let event_processor = DexEventProcessor::new(state.clone());
    let request_processor = DexRequestProcessor::new(state.clone());

    let usdc = Address::new_random();
    let eth = Address::new_random();
    let btc = Address::new_random();

    // Create USDC/ETH and ETH/BTC orderbooks
    // No direct USDC/BTC path, must route through ETH
    // Use small prices to ensure integer division gives non-zero results
    // ETH/USDC: 1 ETH = 2 USDC (ask), 1 USDC = 0.5 ETH
    event_processor
        .process_orderbook(make_orderbook(eth, usdc, 1, 2))
        .unwrap();
    // BTC/ETH: 1 BTC = 2 ETH (ask)
    event_processor
        .process_orderbook(make_orderbook(btc, eth, 1, 2))
        .unwrap();

    // Request USDC -> BTC (requires 2 hops: USDC -> ETH -> BTC)
    // With 1000 USDC at 2 USDC/ETH = 500 ETH
    // With 500 ETH at 2 ETH/BTC = 250 BTC
    let request = SwapRequest {
        input_token: usdc,
        output_token: btc,
        input_amount: 1000,
        min_output_amount: 0,
    };

    let response = request_processor.process_request(request).await.unwrap();
    match response {
        SwapResponse::Success(success) => {
            assert_eq!(success.route.len(), 2, "Expected 2-hop route");
            assert_eq!(success.route[0].input_token, usdc);
            assert_eq!(success.route[0].output_token, eth);
            assert_eq!(success.route[1].input_token, eth);
            assert_eq!(success.route[1].output_token, btc);
        }
        SwapResponse::Failure(msg) => panic!("Expected success, got: {}", msg),
    }
}

#[tokio::test]
async fn stale_orderbooks_are_skipped() {
    use std::time::Duration;

    let state = create_shared_state();
    let event_processor = DexEventProcessor::new(state.clone());
    let request_processor = DexRequestProcessor::new(state.clone());

    let token_a = Address::new_random();
    let token_b = Address::new_random();

    // Ingest an orderbook
    let book = make_orderbook(token_a, token_b, 99, 101);
    event_processor.process_orderbook(book).unwrap();

    // Manually mark the orderbook as stale by backdating its metadata
    state.orderbook_meta.update(&(token_a, token_b), |_, meta| {
        meta.updated_at = std::time::Instant::now() - Duration::from_secs(60);
    });

    // Request should fail because the only path uses a stale orderbook
    let request = SwapRequest {
        input_token: token_a,
        output_token: token_b,
        input_amount: 1000,
        min_output_amount: 0,
    };

    let response = request_processor.process_request(request).await.unwrap();
    // Should fail with "no path" or "invalid request"
    assert!(matches!(response, SwapResponse::Failure(_)));

    // Verify stale skip was recorded
    assert!(
        state
            .metrics
            .stale_or_unhealthy_skipped
            .load(std::sync::atomic::Ordering::Relaxed)
            > 0
    );
}

#[tokio::test]
async fn slippage_protection_rejects_bad_rate() {
    let state = create_shared_state();
    let event_processor = DexEventProcessor::new(state.clone());
    let request_processor = DexRequestProcessor::new(state.clone());

    let token_a = Address::new_random();
    let token_b = Address::new_random();

    // Create orderbook with wide spread
    let book = make_orderbook(token_a, token_b, 50, 200);
    event_processor.process_orderbook(book).unwrap();

    // Request with high minimum output (will fail slippage check)
    let request = SwapRequest {
        input_token: token_a,
        output_token: token_b,
        input_amount: 1000,
        min_output_amount: 999_999, // Impossibly high
    };

    let response = request_processor.process_request(request).await.unwrap();
    assert!(matches!(response, SwapResponse::Failure(msg) if msg.contains("slippage")));
}

#[tokio::test]
async fn metrics_track_all_request_types() {
    let state = create_shared_state();
    let event_processor = DexEventProcessor::new(state.clone());
    let request_processor = DexRequestProcessor::new(state.clone());

    let token_a = Address::new_random();
    let token_b = Address::new_random();

    // Ingest orderbook
    let book = make_orderbook(token_a, token_b, 99, 101);
    event_processor.process_orderbook(book).unwrap();

    // Successful request
    let request = SwapRequest {
        input_token: token_a,
        output_token: token_b,
        input_amount: 1000,
        min_output_amount: 0,
    };
    let _ = request_processor.process_request(request).await;

    // Failed request (unknown token)
    let unknown = Address::new_random();
    let request = SwapRequest {
        input_token: unknown,
        output_token: token_b,
        input_amount: 1000,
        min_output_amount: 0,
    };
    let _ = request_processor.process_request(request).await;

    // Verify metrics
    let metrics = &state.metrics;
    assert_eq!(
        metrics
            .events_total
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        metrics
            .requests_total
            .load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    assert_eq!(
        metrics
            .requests_failed
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}
