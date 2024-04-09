use std::pin::Pin;

use futures::{channel::mpsc::UnboundedSender, Stream};

use super::{
    BalanceUpdate, BalanceUpdateStream, OrderbookUpdate, OrderbookUpdateStream, TradeOrderUpdate,
    TradeOrderUpdateStream,
};

pub struct UpdateStreams {
    pub balance: BalanceUpdateStream,
    pub orderbook: OrderbookUpdateStream,
    pub trade_order: TradeOrderUpdateStream,
}

#[derive(Debug, Clone)]
pub enum MarketUpdate {
    Balance(BalanceUpdate),       // indicates that the balance has changed
    Orderbook(OrderbookUpdate),   // indicates that the orderbook has changed
    TradeOrder(TradeOrderUpdate), // indicates that a trade order has changed, as well as the pair balances
}

impl MarketUpdate {
    pub fn timestamp_millis(&self) -> u64 {
        match self {
            MarketUpdate::Balance(update) => update.timestamp_millis,
            MarketUpdate::Orderbook(update) => update.timestamp_millis(),
            MarketUpdate::TradeOrder(update) => update.timestamp_millis(),
        }
    }
}

pub type MarketUpdateSink = UnboundedSender<MarketUpdate>;
pub type MarketUpdateStream = Pin<Box<dyn Stream<Item = MarketUpdate> + Send>>;
