use std::{cmp::Reverse, collections::BTreeMap};

use arbitrary::Unstructured;

use crate::{
    orderbook::{OrderbookLevel, OrderbookState},
    types::{Address, Price, Quantity},
};

pub struct ClobAgent {
    pairs: Vec<(Address, Address)>,

    /// The true prices of the orderbook levels.
    /// Orderbook depths are generated from these prices.
    true_prices: Vec<Price>,

    min_price: Price,
    max_price: Price,
    level_quantity: Quantity,
}

impl ClobAgent {
    pub fn new(
        pairs: Vec<(Address, Address)>,
        num_prices: usize,
        min_price: Price,
        level_quantity: Quantity,
    ) -> Self {
        let initial_price = min_price + (num_prices as u64 / 2);

        Self {
            pairs,
            true_prices: (0..num_prices).map(|_| initial_price as Price).collect(),
            min_price,
            max_price: min_price + (num_prices - 1) as Price,
            level_quantity,
        }
    }

    pub fn generate_clob(&mut self, u: &mut Unstructured) -> OrderbookState {
        // Pick a random pair
        let pair_idx = u.int_in_range(0..=self.pairs.len() - 1).unwrap();
        let pair = self.pairs[pair_idx];
        let price = self.true_prices[pair_idx];

        let add_one = if price == self.min_price {
            true
        } else if price == self.max_price {
            false
        } else {
            u.int_in_range(0..=1).unwrap() == 0
        };

        let new_price = if add_one { price + 1 } else { price - 1 };
        self.true_prices[pair_idx] = new_price;

        let offset_unit = self.level_quantity / self.true_prices.len() as u64;

        // Generate bids

        let mut bids = BTreeMap::new();
        let mut bid_price = self.min_price;
        let mut i = 0;
        loop {
            if bid_price >= new_price {
                break;
            }

            bids.insert(
                Reverse(bid_price),
                OrderbookLevel {
                    px: bid_price,
                    sz: self.level_quantity + (offset_unit * i as u64),
                },
            );
            bid_price += 1;
            i += 1;
        }

        let mut asks: BTreeMap<Price, OrderbookLevel> = BTreeMap::new();
        let mut ask_price = self.max_price;
        let mut i = 0;
        loop {
            if ask_price <= new_price {
                break;
            }

            asks.insert(
                ask_price,
                OrderbookLevel {
                    px: ask_price,
                    sz: self.level_quantity + (offset_unit * i as u64),
                },
            );
            ask_price -= 1;
            i += 1;
        }

        OrderbookState {
            base_token: pair.0,
            quote_token: pair.1,
            bids,
            asks,
        }
    }
}
