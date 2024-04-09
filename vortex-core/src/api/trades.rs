use anyhow::Error;
use async_trait::async_trait;

use crate::types::{TradeOrderUpdateStream, TradeOrders};

#[async_trait]
pub trait OrderFetcher {
    /// Get the pending orders for a given pair, ordered by price (sells are ascending, buys are descending)
    async fn get_orders_pending(&self, pair: &str) -> Result<TradeOrders, Error>;

    async fn subscribe_orders_pending(&self, pair: &str) -> Result<TradeOrderUpdateStream, Error>;
}
