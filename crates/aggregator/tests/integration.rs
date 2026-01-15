//! Integration tests for the DEX aggregator.

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

    let book = make_orderbook(token_a, token_b, 99, 101);
    event_processor.process_orderbook(book).unwrap();

    let request = SwapRequest {
        input_token: token_a,
        output_token: token_b,
        input_amount: 1000,
        min_output_amount: 0,
    };

    let response = request_processor.process_request(request).await.unwrap();
    assert!(matches!(response, SwapResponse::Success(_)));

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
async fn multi_hop_routing_across_orderbooks() {
    let state = create_shared_state();
    let event_processor = DexEventProcessor::new(state.clone());
    let request_processor = DexRequestProcessor::new(state.clone());

    let usdc = Address::new_random();
    let eth = Address::new_random();
    let btc = Address::new_random();

    event_processor
        .process_orderbook(make_orderbook(eth, usdc, 1, 2))
        .unwrap();
    event_processor
        .process_orderbook(make_orderbook(btc, eth, 1, 2))
        .unwrap();

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

    let book = make_orderbook(token_a, token_b, 99, 101);
    event_processor.process_orderbook(book).unwrap();

    // Mark orderbook as stale
    state.orderbook_health.update(&(token_a, token_b), |_, health| {
        health.last_updated = std::time::Instant::now() - Duration::from_secs(60);
    });

    let request = SwapRequest {
        input_token: token_a,
        output_token: token_b,
        input_amount: 1000,
        min_output_amount: 0,
    };

    let response = request_processor.process_request(request).await.unwrap();
    assert!(matches!(response, SwapResponse::Failure(_)));

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

    let book = make_orderbook(token_a, token_b, 50, 200);
    event_processor.process_orderbook(book).unwrap();

    let request = SwapRequest {
        input_token: token_a,
        output_token: token_b,
        input_amount: 1000,
        min_output_amount: 999_999,
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
