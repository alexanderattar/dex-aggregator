use aggregator_utils::orderbook::OrderbookState;
use aggregator_utils::types::Side;

// Simulates filling against an orderbook. Doesn't execute trades,
// just calculates expected output for routing decisions.
pub struct MatchResult {
    pub input_consumed: u64, // may be less than requested if orderbook runs dry
    pub output_produced: u64,
}

// Match an order against an orderbook.
pub fn match_order(orderbook: &OrderbookState, side: Side, input_amount: u64) -> MatchResult {
    match side {
        Side::Ask => match_asks(orderbook, input_amount),
        Side::Bid => match_bids(orderbook, input_amount),
    }
}

// Buy base token by spending quote. Walks asks from cheapest up.
fn match_asks(orderbook: &OrderbookState, quote_to_spend: u64) -> MatchResult {
    // Initialize the remaining amount
    let mut remaining = quote_to_spend;
    // Initialize the output amount
    let mut base_out = 0u64;

    // Buy base token by spending quote. Walks asks from cheapest up.
    for level in orderbook.asks() {
        if remaining == 0 || level.px == 0 {
            break;
        }

        // How much base can we buy at this price?
        let affordable = remaining / level.px;
        let take = affordable.min(level.sz);

        // u128 intermediate to avoid overflow on price * qty
        let cost = (take as u128)
            .saturating_mul(level.px as u128)
            .min(u64::MAX as u128) as u64;

        // Update the output amount
        base_out = base_out.saturating_add(take);
        // Update the remaining amount
        remaining = remaining.saturating_sub(cost);
    }

    MatchResult {
        input_consumed: quote_to_spend - remaining,
        output_produced: base_out,
    }
}

// Sell base token for quote. Walks bids from best price down.
fn match_bids(orderbook: &OrderbookState, base_to_sell: u64) -> MatchResult {
    // Initialize the remaining amount
    let mut remaining = base_to_sell;
    // Initialize the output amount
    let mut quote_out = 0u64;

    // Sell base token for quote. Walks bids from best price down.
    for level in orderbook.bids() {
        if remaining == 0 {
            break;
        }

        // How much quote can we get for this base?
        let take = remaining.min(level.sz);

        // u128 intermediate to avoid overflow on price * qty
        let received = (take as u128)
            .saturating_mul(level.px as u128)
            .min(u64::MAX as u128) as u64;

        // Update the output amount
        quote_out = quote_out.saturating_add(received);
        // Update the remaining amount
        remaining = remaining.saturating_sub(take);
    }

    MatchResult {
        input_consumed: base_to_sell - remaining,
        output_produced: quote_out,
    }
}
