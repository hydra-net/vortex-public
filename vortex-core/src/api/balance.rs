use std::collections::HashMap;

use anyhow::Error;
use async_trait::async_trait;

use crate::types::{Balance, BalanceUpdateStream};

#[async_trait]
pub trait BalanceFetcher {
    async fn get_balances(&self) -> Result<HashMap<String, Balance>, Error>;

    async fn subscribe_balances(&self) -> Result<BalanceUpdateStream, Error>;
}
