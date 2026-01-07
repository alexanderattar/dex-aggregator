use arbitrary::Unstructured;

use crate::types::{Address, SwapRequest};

/// A request agent randomly generates swap requests.
pub struct RequestAgent {
    tokens: Vec<Address>,
    slippage_tolerance_bps: u16,
}

impl RequestAgent {
    pub fn new(tokens: Vec<Address>, slippage_tolerance_bps: u16) -> Self {
        Self {
            tokens,
            slippage_tolerance_bps,
        }
    }

    pub fn generate_request(&self, u: &mut Unstructured) -> SwapRequest {
        let input_token_idx = u.int_in_range(0..=self.tokens.len() - 1).unwrap();
        let output_token_idx = u.int_in_range(0..=self.tokens.len() - 2).unwrap();

        let input_token = self.tokens[input_token_idx];

        let output_token = if input_token_idx > output_token_idx {
            self.tokens[output_token_idx]
        } else {
            self.tokens[output_token_idx + 1]
        };

        assert!(input_token != output_token);

        let input_amount: u64 = u.int_in_range(1..=1_000_000).unwrap();
        let min_output_amount: u64 =
            input_amount * (10000 - self.slippage_tolerance_bps as u64) / 10000;

        SwapRequest {
            input_token,
            output_token,
            input_amount,
            min_output_amount,
        }
    }
}
