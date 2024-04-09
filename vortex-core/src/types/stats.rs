use std::pin::Pin;

use futures::Stream;
use rust_decimal::Decimal;

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub period: u64,
    pub last: Decimal,
    pub open: Decimal,
    pub close: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub usd_volume: Decimal,
    pub quote_volume: Decimal,
}

impl Stats {
    pub(crate) fn update(&mut self, update: &Stats) {
        *self = update.clone();
    }
}

#[derive(Debug, Clone)]
pub struct StatsUpdate {
    pub pair: String,
    pub stats: Stats,
}

pub type StatsUpdateStream = Pin<Box<dyn Stream<Item = StatsUpdate> + Send>>;
