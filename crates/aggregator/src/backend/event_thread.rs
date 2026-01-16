use std::time::Duration;

use aggregator_utils::{clob_agent::ClobAgent, types::Address};
use arbitrary::Unstructured;
use async_trait::async_trait;
use tokio::time::interval;
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemHandle};
use tracing::{debug, trace};

use crate::traits::EventProcessor;

pub struct EventThreadConfig {
    pub events_per_second: u32,
    pub num_prices: usize,
    pub pairs: Vec<(Address, Address)>,
}

/// A subsystem simulating an incoming stream of orderbook events.
/// In reality, these events would be coming from an EVM sequencer, as logs
/// emitted from a smart contract.
pub struct EventThread<P: EventProcessor> {
    events_per_second: Duration,
    clob_agent: ClobAgent,
    processor: P,
}

impl<P: EventProcessor> EventThread<P> {
    pub fn new(config: EventThreadConfig, processor: P) -> Self {
        let events_per_second = Duration::from_secs(1) / config.events_per_second;
        let clob_agent = ClobAgent::new(config.pairs, config.num_prices, 5, 100_000);

        Self {
            events_per_second,
            clob_agent,
            processor,
        }
    }

    async fn thread_loop(mut self, subsys: SubsystemHandle) -> Result<(), P::Error> {
        let mut ticker = interval(self.events_per_second);

        loop {
            tokio::select! {
                _ = subsys.on_shutdown_requested() => {
                    trace!("EventThread shutdown requested");
                    break;
                }

                _ = ticker.tick() => {
                    let input_data: [u8; 32] = rand::random();
                    let mut u = Unstructured::new(&input_data);
                    let orderbook = self.clob_agent.generate_clob(&mut u);
                    self.processor.process_orderbook(orderbook)?;
                }
            }
        }

        debug!("EventThread shutting down");
        Ok(())
    }
}

#[async_trait]
impl<P: EventProcessor> IntoSubsystem<P::Error> for EventThread<P> {
    async fn run(self, subsys: SubsystemHandle) -> Result<(), P::Error> {
        self.thread_loop(subsys).await
    }
}
