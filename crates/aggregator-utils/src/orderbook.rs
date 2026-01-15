use std::{cmp::Reverse, collections::BTreeMap};

use crate::types::{Address, Price, Quantity, Side};

#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub side: Side,
    pub price: Price,
    pub size: Quantity,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderbookLevel {
    // price
    pub px: Price,
    // size
    pub sz: Quantity,
}

#[derive(Debug, Clone)]
pub struct OrderbookState {
    pub base_token: Address,
    pub quote_token: Address,

    pub(crate) bids: BTreeMap<Reverse<Price>, OrderbookLevel>,
    pub(crate) asks: BTreeMap<Price, OrderbookLevel>,
}

impl OrderbookState {
    pub fn new(base_token: Address, quote_token: Address) -> Self {
        Self {
            base_token,
            quote_token,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
        }
    }

    pub fn asks(&self) -> Vec<&OrderbookLevel> {
        self.asks.values().collect()
    }

    pub fn bids(&self) -> Vec<&OrderbookLevel> {
        self.bids.values().collect()
    }

    pub fn insert_bid(&mut self, level: OrderbookLevel) {
        self.bids.insert(Reverse(level.px), level);
    }

    pub fn insert_ask(&mut self, level: OrderbookLevel) {
        self.asks.insert(level.px, level);
    }
}
