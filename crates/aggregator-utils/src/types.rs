use std::fmt::Display;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address([u8; 20]);

impl Address {
    pub fn new_random() -> Self {
        Self(rand::random())
    }
}

impl std::fmt::Debug for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

impl Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwapRequest {
    pub input_token: Address,
    pub output_token: Address,
    pub input_amount: Quantity,
    pub min_output_amount: Quantity,
}

pub type Price = u64;

pub type Quantity = u64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Bid,
    Ask,
}

#[derive(Debug, Clone)]
pub struct Swap {
    pub input_token: Address,
    pub output_token: Address,

    /// The direction of the swap to start from. (e.g. bid for buys, ask for sells)
    pub direction: Side,

    /// The amount of input tokens used in the swap.
    pub input_amount: Quantity,

    /// The amount of output tokens received in the swap.
    pub expected_output_amount: Quantity,
}

pub enum SwapResponse {
    /// The swap was successful, return the swap details
    Success(SwapResponseSuccess),

    /// The swap failed, return a reason
    Failure(String),
}

pub struct SwapResponseSuccess {
    /// The route of the swap, from input token to output token.
    /// For example, for a two-hop swap from USDC -> DOGE -> WETH,
    /// the route would be:
    /// [(usdc, doge), (doge, weth)]
    /// where the route.first().input_amount is the input amount requested from the user,
    /// and route.last().expected_output_amount is the expected output amount of the entire swap.
    pub route: Vec<Swap>,
}
