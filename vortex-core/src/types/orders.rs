use rust_decimal::Decimal;

use super::TradeOrder;

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct Order {
    pub price: Decimal,
    pub size: Decimal,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Side {
    Buy,
    Sell,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Buy => write!(f, "buy"),
            Side::Sell => write!(f, "sell"),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Type {
    Limit,
    Market,
}

impl From<TradeOrder> for Order {
    fn from(order: TradeOrder) -> Self {
        Order {
            price: order.price,
            size: order.amount,
        }
    }
}
