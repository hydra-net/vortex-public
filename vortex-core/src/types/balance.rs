use std::pin::Pin;

use futures::{channel::mpsc::UnboundedSender, Stream};
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balance {
    pub free: Decimal,
    pub used: Decimal,
    pub total: Decimal,
    pub remote_free: Decimal,
    pub remote_used: Decimal,
    pub remote_total: Decimal,
}

impl Default for Balance {
    fn default() -> Self {
        Self {
            free: Decimal::ZERO,
            used: Decimal::ZERO,
            total: Decimal::ZERO,
            remote_free: Decimal::MAX,
            remote_used: Decimal::ZERO,
            remote_total: Decimal::MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceUpdate {
    pub timestamp_millis: u64,
    pub currency: String,
    pub balance: Balance,
}

pub type BalanceUpdateSink = UnboundedSender<BalanceUpdate>;
pub type BalanceUpdateStream = Pin<Box<dyn Stream<Item = BalanceUpdate> + Send>>;
