use std::pin::Pin;

use futures::{channel::mpsc::UnboundedSender, Stream};
use rust_decimal::Decimal;

use super::{Side, Type};

#[derive(Debug, Default, PartialEq, Clone)]
pub struct TradeOrders {
    pub buys: Vec<TradeOrder>,
    pub sells: Vec<TradeOrder>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeOrder {
    pub id: String,
    pub timestamp_millis: u64,
    pub side: Side,
    pub r#type: Type,
    pub price: Decimal,    // in quote coin
    pub amount: Decimal,   // order amount in base coin
    pub left: Decimal,     // left amount in base coin
    pub executed: Decimal, // executed amount in base coin
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradeOrderUpdate {
    Added(TradeOrder),
    Changed(TradeOrder),
    Completed {
        order_id: String,
        timestamp_millis: u64,
    },
    Canceled {
        order_id: String,
        timestamp_millis: u64,
    },
}

impl TradeOrderUpdate {
    pub fn order_id(&self) -> &str {
        match self {
            TradeOrderUpdate::Added(order) => &order.id,
            TradeOrderUpdate::Changed(order) => &order.id,
            TradeOrderUpdate::Completed { order_id, .. } => order_id,
            TradeOrderUpdate::Canceled { order_id, .. } => order_id,
        }
    }

    pub fn timestamp_millis(&self) -> u64 {
        match self {
            TradeOrderUpdate::Added(order) => order.timestamp_millis,
            TradeOrderUpdate::Changed(order) => order.timestamp_millis,
            TradeOrderUpdate::Completed {
                timestamp_millis, ..
            } => *timestamp_millis,
            TradeOrderUpdate::Canceled {
                timestamp_millis, ..
            } => *timestamp_millis,
        }
    }
}

pub type TradeOrderUpdateSink = UnboundedSender<TradeOrderUpdate>;
pub type TradeOrderUpdateStream = Pin<Box<dyn Stream<Item = TradeOrderUpdate> + Send>>;

impl TradeOrders {
    pub fn get_order_by_id(&self, id: &str) -> Option<&TradeOrder> {
        // find order by id iterating in self.buys and self.sells
        self.buys
            .iter()
            .find(|o| o.id == id)
            .or_else(|| self.sells.iter().find(|o| o.id == id))
    }
    pub fn get_highest_buy(&self) -> Option<&TradeOrder> {
        self.buys.first()
    }
    pub fn get_lowest_sell(&self) -> Option<&TradeOrder> {
        self.sells.first()
    }
    pub fn get_lowest_buy(&self) -> Option<&TradeOrder> {
        self.buys.last()
    }
    pub fn get_highest_sell(&self) -> Option<&TradeOrder> {
        self.sells.last()
    }
    pub fn insert_order(&mut self, order: TradeOrder) {
        if order.side == Side::Buy {
            self.insert_buy(order);
        } else {
            self.insert_sell(order);
        }
    }
    fn insert_buy(&mut self, order: TradeOrder) {
        // insert buy order at the right position
        let mut i = 0;
        while i < self.buys.len() && order.price < self.buys[i].price {
            i += 1;
        }
        if self.buys.get(i).is_some() && order.id == self.buys[i].id {
            self.buys[i] = order;
        } else {
            self.buys.insert(i, order);
        }
    }
    fn insert_sell(&mut self, order: TradeOrder) {
        // insert sell order at the right position
        let mut i = 0;
        while i < self.sells.len() && order.price > self.sells[i].price {
            i += 1;
        }
        if self.sells.get(i).is_some() && order.id == self.sells[i].id {
            self.sells[i] = order;
        } else {
            self.sells.insert(i, order);
        }
    }
    pub fn remove_order(&mut self, order: TradeOrder) {
        if order.side == Side::Buy {
            self.remove_buy(order);
        } else {
            self.remove_sell(order);
        }
    }
    pub fn remove_order_by_id(&mut self, id: &str) -> Option<TradeOrder> {
        // remove order by id
        if let Some(order) = self.get_order_by_id(id).cloned() {
            if order.side == Side::Buy {
                self.remove_buy(order.clone());
            } else {
                self.remove_sell(order.clone());
            }
            Some(order)
        } else {
            None
        }
    }
    fn remove_buy(&mut self, order: TradeOrder) {
        self.buys.retain(|o| o.id != order.id);
    }
    fn remove_sell(&mut self, order: TradeOrder) {
        self.sells.retain(|o| o.id != order.id);
    }
    pub(crate) fn update_order(&mut self, order: TradeOrder) {
        if order.side == Side::Buy {
            self.update_buy(order);
        } else {
            self.update_sell(order);
        }
    }
    fn update_buy(&mut self, order: TradeOrder) {
        // update buy order
        let mut i = 0;
        while i < self.buys.len() && self.buys[i].id != order.id {
            i += 1;
        }
        if i < self.buys.len() {
            self.buys[i] = order;
        }
    }
    fn update_sell(&mut self, order: TradeOrder) {
        // update sell order
        let mut i = 0;
        while i < self.sells.len() && self.sells[i].id != order.id {
            i += 1;
        }
        if i < self.sells.len() {
            self.sells[i] = order;
        }
    }
    pub(crate) fn push_order(&mut self, order: TradeOrder) {
        if order.side == Side::Buy {
            self.buys.push(order);
        } else {
            self.sells.push(order);
        }
    }
}

#[cfg(test)]
mod test {
    use rust_decimal::prelude::FromPrimitive;

    use super::*;
    use crate::types::Type;

    fn prepare_tradeorders() -> TradeOrders {
        TradeOrders {
            buys: vec![
                TradeOrder {
                    id: "buy1".to_string(),
                    timestamp_millis: 1,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(990.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "buy2".to_string(),
                    timestamp_millis: 2,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(980.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
            ],
            sells: vec![
                TradeOrder {
                    id: "sell1".to_string(),
                    timestamp_millis: 1,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1010.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "sell2".to_string(),
                    timestamp_millis: 2,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1020.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
            ],
        }
    }

    #[test]
    fn test_get_order_by_id() {
        let tradeorders = prepare_tradeorders();
        assert_eq!(
            tradeorders.get_order_by_id("buy1"),
            Some(&TradeOrder {
                id: "buy1".to_string(),
                timestamp_millis: 1,
                side: Side::Buy,
                r#type: Type::Limit,
                price: Decimal::from_f64(990.0).unwrap(),
                amount: Decimal::from_f64(10.0).unwrap(),
                left: Decimal::from_f64(10.0).unwrap(),
                executed: Decimal::ZERO,
            })
        );
        assert_eq!(
            tradeorders.get_order_by_id("sell2"),
            Some(&TradeOrder {
                id: "sell2".to_string(),
                timestamp_millis: 2,
                side: Side::Sell,
                r#type: Type::Limit,
                price: Decimal::from_f64(1020.0).unwrap(),
                amount: Decimal::from_f64(10.0).unwrap(),
                left: Decimal::from_f64(10.0).unwrap(),
                executed: Decimal::ZERO,
            })
        );
        assert_eq!(tradeorders.get_order_by_id("buy3"), None);
        assert_eq!(tradeorders.get_order_by_id("sell3"), None);
    }

    #[test]
    fn test_get_highest_buy() {
        let tradeorders = prepare_tradeorders();
        assert_eq!(
            tradeorders.get_highest_buy(),
            Some(&TradeOrder {
                id: "buy1".to_string(),
                timestamp_millis: 1,
                side: Side::Buy,
                r#type: Type::Limit,
                price: Decimal::from_f64(990.0).unwrap(),
                amount: Decimal::from_f64(10.0).unwrap(),
                left: Decimal::from_f64(10.0).unwrap(),
                executed: Decimal::ZERO,
            })
        );
    }

    #[test]
    fn test_get_lowest_sell() {
        let tradeorders = prepare_tradeorders();
        assert_eq!(
            tradeorders.get_lowest_sell(),
            Some(&TradeOrder {
                id: "sell1".to_string(),
                timestamp_millis: 1,
                side: Side::Sell,
                r#type: Type::Limit,
                price: Decimal::from_f64(1010.0).unwrap(),
                amount: Decimal::from_f64(10.0).unwrap(),
                left: Decimal::from_f64(10.0).unwrap(),
                executed: Decimal::ZERO,
            })
        );
    }

    #[test]
    fn test_get_highest_sell() {
        let tradeorders = prepare_tradeorders();
        assert_eq!(
            tradeorders.get_highest_sell(),
            Some(&TradeOrder {
                id: "sell2".to_string(),
                timestamp_millis: 2,
                side: Side::Sell,
                r#type: Type::Limit,
                price: Decimal::from_f64(1020.0).unwrap(),
                amount: Decimal::from_f64(10.0).unwrap(),
                left: Decimal::from_f64(10.0).unwrap(),
                executed: Decimal::ZERO,
            })
        );
    }

    #[test]
    fn test_get_lowest_buy() {
        let tradeorders = prepare_tradeorders();
        assert_eq!(
            tradeorders.get_lowest_buy(),
            Some(&TradeOrder {
                id: "buy2".to_string(),
                timestamp_millis: 2,
                side: Side::Buy,
                r#type: Type::Limit,
                price: Decimal::from_f64(980.0).unwrap(),
                amount: Decimal::from_f64(10.0).unwrap(),
                left: Decimal::from_f64(10.0).unwrap(),
                executed: Decimal::ZERO,
            })
        );
    }

    #[test]
    fn test_insert_order() {
        let mut tradeorders = prepare_tradeorders();

        tradeorders.insert_order(TradeOrder {
            id: "buy3".to_string(),
            timestamp_millis: 3,
            side: Side::Buy,
            r#type: Type::Limit,
            price: Decimal::from_f64(990.0).unwrap(),
            amount: Decimal::from_f64(10.0).unwrap(),
            left: Decimal::from_f64(10.0).unwrap(),
            executed: Decimal::ZERO,
        });
        tradeorders.insert_order(TradeOrder {
            id: "buy4".to_string(),
            timestamp_millis: 4,
            side: Side::Buy,
            r#type: Type::Limit,
            price: Decimal::from_f64(970.0).unwrap(),
            amount: Decimal::from_f64(20.0).unwrap(),
            left: Decimal::from_f64(20.0).unwrap(),
            executed: Decimal::ZERO,
        });
        tradeorders.insert_order(TradeOrder {
            id: "sell3".to_string(),
            timestamp_millis: 3,
            side: Side::Sell,
            r#type: Type::Limit,
            price: Decimal::from_f64(1010.0).unwrap(),
            amount: Decimal::from_f64(10.0).unwrap(),
            left: Decimal::from_f64(10.0).unwrap(),
            executed: Decimal::ZERO,
        });
        tradeorders.insert_order(TradeOrder {
            id: "sell4".to_string(),
            timestamp_millis: 4,
            side: Side::Sell,
            r#type: Type::Limit,
            price: Decimal::from_f64(1030.0).unwrap(),
            amount: Decimal::from_f64(20.0).unwrap(),
            left: Decimal::from_f64(20.0).unwrap(),
            executed: Decimal::ZERO,
        });

        assert_eq!(
            tradeorders.buys,
            vec![
                TradeOrder {
                    id: "buy3".to_string(),
                    timestamp_millis: 3,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(990.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "buy1".to_string(),
                    timestamp_millis: 1,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(990.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "buy2".to_string(),
                    timestamp_millis: 2,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(980.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "buy4".to_string(),
                    timestamp_millis: 4,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(970.0).unwrap(),
                    amount: Decimal::from_f64(20.0).unwrap(),
                    left: Decimal::from_f64(20.0).unwrap(),
                    executed: Decimal::ZERO,
                },
            ]
        );

        assert_eq!(
            tradeorders.sells,
            vec![
                TradeOrder {
                    id: "sell3".to_string(),
                    timestamp_millis: 3,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1010.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "sell1".to_string(),
                    timestamp_millis: 1,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1010.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "sell2".to_string(),
                    timestamp_millis: 2,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1020.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "sell4".to_string(),
                    timestamp_millis: 4,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1030.0).unwrap(),
                    amount: Decimal::from_f64(20.0).unwrap(),
                    left: Decimal::from_f64(20.0).unwrap(),
                    executed: Decimal::ZERO,
                },
            ]
        );
    }

    #[test]
    fn test_remove_order() {
        let mut tradeorders = prepare_tradeorders();

        tradeorders.remove_order(TradeOrder {
            id: "sell1".to_string(),
            timestamp_millis: 1,
            side: Side::Sell,
            r#type: Type::Limit,
            price: Decimal::from_f64(1010.0).unwrap(),
            amount: Decimal::from_f64(10.0).unwrap(),
            left: Decimal::from_f64(10.0).unwrap(),
            executed: Decimal::ZERO,
        });
        tradeorders.remove_order(TradeOrder {
            id: "buy2".to_string(),
            timestamp_millis: 2,
            side: Side::Buy,
            r#type: Type::Limit,
            price: Decimal::from_f64(980.0).unwrap(),
            amount: Decimal::from_f64(10.0).unwrap(),
            left: Decimal::from_f64(10.0).unwrap(),
            executed: Decimal::ZERO,
        });

        assert_eq!(
            tradeorders.buys,
            vec![TradeOrder {
                id: "buy1".to_string(),
                timestamp_millis: 1,
                side: Side::Buy,
                r#type: Type::Limit,
                price: Decimal::from_f64(990.0).unwrap(),
                amount: Decimal::from_f64(10.0).unwrap(),
                left: Decimal::from_f64(10.0).unwrap(),
                executed: Decimal::ZERO,
            },]
        );
        assert_eq!(
            tradeorders.sells,
            vec![TradeOrder {
                id: "sell2".to_string(),
                timestamp_millis: 2,
                side: Side::Sell,
                r#type: Type::Limit,
                price: Decimal::from_f64(1020.0).unwrap(),
                amount: Decimal::from_f64(10.0).unwrap(),
                left: Decimal::from_f64(10.0).unwrap(),
                executed: Decimal::ZERO,
            },]
        );
    }

    #[test]
    fn test_remove_order_by_id() {
        let mut tradeorders = prepare_tradeorders();

        tradeorders.remove_order_by_id("sell2");
        tradeorders.remove_order_by_id("buy1");

        assert_eq!(
            tradeorders.buys,
            vec![TradeOrder {
                id: "buy2".to_string(),
                timestamp_millis: 2,
                side: Side::Buy,
                r#type: Type::Limit,
                price: Decimal::from_f64(980.0).unwrap(),
                amount: Decimal::from_f64(10.0).unwrap(),
                left: Decimal::from_f64(10.0).unwrap(),
                executed: Decimal::ZERO,
            },]
        );
        assert_eq!(
            tradeorders.sells,
            vec![TradeOrder {
                id: "sell1".to_string(),
                timestamp_millis: 1,
                side: Side::Sell,
                r#type: Type::Limit,
                price: Decimal::from_f64(1010.0).unwrap(),
                amount: Decimal::from_f64(10.0).unwrap(),
                left: Decimal::from_f64(10.0).unwrap(),
                executed: Decimal::ZERO,
            },]
        );
    }

    #[test]
    fn test_update_order() {
        let mut tradeorders = prepare_tradeorders();

        tradeorders.update_order(TradeOrder {
            id: "sell1".to_string(),
            timestamp_millis: 1,
            side: Side::Sell,
            r#type: Type::Limit,
            price: Decimal::from_f64(1010.0).unwrap(),
            amount: Decimal::from_f64(10.0).unwrap(),
            left: Decimal::from_f64(6.0).unwrap(),
            executed: Decimal::from_f64(4.0).unwrap(),
        });
        tradeorders.update_order(TradeOrder {
            id: "buy2".to_string(),
            timestamp_millis: 2,
            side: Side::Buy,
            r#type: Type::Limit,
            price: Decimal::from_f64(980.0).unwrap(),
            amount: Decimal::from_f64(20.0).unwrap(),
            left: Decimal::from_f64(20.0).unwrap(),
            executed: Decimal::ZERO,
        });

        assert_eq!(
            tradeorders.buys,
            vec![
                TradeOrder {
                    id: "buy1".to_string(),
                    timestamp_millis: 1,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(990.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "buy2".to_string(),
                    timestamp_millis: 2,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(980.0).unwrap(),
                    amount: Decimal::from_f64(20.0).unwrap(),
                    left: Decimal::from_f64(20.0).unwrap(),
                    executed: Decimal::ZERO,
                },
            ]
        );

        assert_eq!(
            tradeorders.sells,
            vec![
                TradeOrder {
                    id: "sell1".to_string(),
                    timestamp_millis: 1,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1010.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(6.0).unwrap(),
                    executed: Decimal::from_f64(4.0).unwrap(),
                },
                TradeOrder {
                    id: "sell2".to_string(),
                    timestamp_millis: 2,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1020.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
            ]
        );
    }

    #[test]
    fn test_push_order() {
        let mut tradeorders = prepare_tradeorders();

        tradeorders.push_order(TradeOrder {
            id: "buy3".to_string(),
            timestamp_millis: 3,
            side: Side::Buy,
            r#type: Type::Limit,
            price: Decimal::from_f64(1000.0).unwrap(),
            amount: Decimal::from_f64(10.0).unwrap(),
            left: Decimal::ZERO,
            executed: Decimal::from_f64(10.0).unwrap(),
        });
        tradeorders.push_order(TradeOrder {
            id: "sell3".to_string(),
            timestamp_millis: 3,
            side: Side::Sell,
            r#type: Type::Limit,
            price: Decimal::from_f64(1000.0).unwrap(),
            amount: Decimal::from_f64(10.0).unwrap(),
            left: Decimal::ZERO,
            executed: Decimal::from_f64(10.0).unwrap(),
        });

        assert_eq!(
            tradeorders.buys,
            vec![
                TradeOrder {
                    id: "buy1".to_string(),
                    timestamp_millis: 1,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(990.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "buy2".to_string(),
                    timestamp_millis: 2,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(980.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "buy3".to_string(),
                    timestamp_millis: 3,
                    side: Side::Buy,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1000.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::ZERO,
                    executed: Decimal::from_f64(10.0).unwrap(),
                },
            ]
        );
        assert_eq!(
            tradeorders.sells,
            vec![
                TradeOrder {
                    id: "sell1".to_string(),
                    timestamp_millis: 1,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1010.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "sell2".to_string(),
                    timestamp_millis: 2,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1020.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::from_f64(10.0).unwrap(),
                    executed: Decimal::ZERO,
                },
                TradeOrder {
                    id: "sell3".to_string(),
                    timestamp_millis: 3,
                    side: Side::Sell,
                    r#type: Type::Limit,
                    price: Decimal::from_f64(1000.0).unwrap(),
                    amount: Decimal::from_f64(10.0).unwrap(),
                    left: Decimal::ZERO,
                    executed: Decimal::from_f64(10.0).unwrap(),
                },
            ]
        );
    }
}
