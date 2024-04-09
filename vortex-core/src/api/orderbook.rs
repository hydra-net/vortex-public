use anyhow::Error;
use async_trait::async_trait;

use crate::types::{Orderbook, OrderbookUpdateStream};

#[async_trait]
pub trait OrderbookFetcher {
    async fn get_orderbook(&self, pair: &str) -> Result<Orderbook, Error>;

    async fn subscribe_orderbook(&self, pair: &str) -> Result<OrderbookUpdateStream, Error>;
}
