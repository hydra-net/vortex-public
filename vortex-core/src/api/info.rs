use anyhow::Error;
use async_trait::async_trait;

use crate::types::Info;

#[async_trait]
pub trait InfoFetcher {
    async fn get_market_info(&self, pair: &str) -> Result<Info, Error>;
}
