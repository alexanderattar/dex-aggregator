use std::time::Duration;

use aggregator_utils::{request_agent::RequestAgent, types::Address};
use arbitrary::Unstructured;
use async_trait::async_trait;
use tokio::time::interval;
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemHandle};
use tracing::{debug, trace};

use crate::traits::RequestProcessor;

pub struct RequestThreadConfig {
    pub requests_per_second: u32,
    pub slippage_tolerance_bps: u16,
    pub tokens: Vec<Address>,
}

/// A subsystem simulating an incoming stream of orderbook events.
/// In reality, these events would be coming from an EVM sequencer, as logs
/// emitted from a smart contract.
pub struct RequestThread<P: RequestProcessor> {
    requests_per_second: Duration,
    request_agent: RequestAgent,
    processor: P,
}

impl<P: RequestProcessor> RequestThread<P> {
    pub fn new(config: RequestThreadConfig, processor: P) -> Self {
        let request_agent = RequestAgent::new(config.tokens, config.slippage_tolerance_bps);
        let requests_per_second = Duration::from_secs(1) / config.requests_per_second;

        Self {
            requests_per_second,
            request_agent,
            processor,
        }
    }

    async fn thread_loop(self, subsys: SubsystemHandle) -> Result<(), P::Error> {
        let mut ticker = interval(self.requests_per_second);

        loop {
            tokio::select! {
                _ = subsys.on_shutdown_requested() => {
                    trace!("RequestThread shutdown requested");
                    break;
                }

                _ = ticker.tick() => {
                    let input_data: [u8; 32] = rand::random();
                    let mut u = Unstructured::new(&input_data);
                    let request = self.request_agent.generate_request(&mut u);

                    self.processor.process_request(request).await?;
                }
            }
        }

        debug!("RequestThread shutting down");
        Ok(())
    }
}

#[async_trait]
impl<P: RequestProcessor> IntoSubsystem<P::Error> for RequestThread<P> {
    async fn run(self, subsys: SubsystemHandle) -> Result<(), P::Error> {
        self.thread_loop(subsys).await
    }
}
