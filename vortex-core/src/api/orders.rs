use anyhow::Error;
use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::types::{Side, TradeOrder};

// TODO: orders in quotes

#[derive(Debug, thiserror::Error)]
pub enum CreateOrderError {
    #[error("Insufficient funds")]
    InsufficientFunds,
    #[error("{0}")]
    Other(Error),
}

#[async_trait]
pub trait OrderCreator {
    /// Create limit order. Amount is in base currency, price is in quote currency.
    async fn create_limit_order(
        &self,
        pair: &str,
        side: Side,
        amount: Decimal,
        price: Decimal,
    ) -> Result<TradeOrder, CreateOrderError>;

    /// Create market order. Amount is in base currency.
    async fn create_market_order(
        &self,
        pair: &str,
        side: Side,
        amount: Decimal,
    ) -> Result<TradeOrder, CreateOrderError>;

    /// Create market order. Amount is in base currency for sell and in quote currency for buy.
    async fn create_hold_market_order(
        &self,
        pair: &str,
        side: Side,
        amount: Decimal,
    ) -> Result<TradeOrder, CreateOrderError>;

    async fn cancel_order(&self, pair: &str, id: &str) -> Result<(), CreateOrderError>;

    async fn cancel_all_orders(&self, pair: &str) -> Result<(), CreateOrderError>;
}
