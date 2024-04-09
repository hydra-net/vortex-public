use anyhow::Error;
use async_trait::async_trait;
use rust_decimal::Decimal;

#[async_trait]
pub trait PriceFeed: Send + Sync + 'static {
    async fn get_price(&self, pair: &str) -> Result<Decimal, Error>;
    async fn get_usd_coin_price(&self, coin: &str) -> Result<Decimal, Error>;
}

#[async_trait]
pub trait OrderbookPriceFeed: PriceFeed {
    async fn get_best_ask(&self, pair: &str) -> Result<Decimal, Error>;
    async fn get_best_bid(&self, pair: &str) -> Result<Decimal, Error>;
}
