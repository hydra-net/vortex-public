use std::pin::Pin;

use futures::{channel::mpsc::UnboundedSender, Stream};
use rust_decimal::{prelude::FromPrimitive, Decimal};

use super::{Order, Side};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Orderbook {
    pub timestamp_millis: u64,
    pub bids: Vec<Order>,
    pub asks: Vec<Order>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderbookUpdate {
    Snapshot {
        timestamp_millis: u64,
        bids: Vec<Order>,
        asks: Vec<Order>,
    },
    OrdersUpdate {
        timestamp_millis: u64,
        orders_update: Vec<OrderUpdate>,
    },
}

impl OrderbookUpdate {
    pub fn timestamp_millis(&self) -> u64 {
        match self {
            OrderbookUpdate::Snapshot {
                timestamp_millis, ..
            } => *timestamp_millis,
            OrderbookUpdate::OrdersUpdate {
                timestamp_millis, ..
            } => *timestamp_millis,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderUpdate {
    pub price: Decimal,
    pub size: Decimal,
    pub side: Side,
    pub update_type: OrderUpdateType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderUpdateType {
    Added,   // new order to be added to an existing orderbook order
    Changed, // existing orderbook order to be changed (null amount means delete)
    Removed, // order to be subtracted from an existing orderbook order
}

pub type OrderbookUpdateSink = UnboundedSender<OrderbookUpdate>;
pub type OrderbookUpdateStream = Pin<Box<dyn Stream<Item = OrderbookUpdate> + Send>>;

impl Orderbook {
    pub fn get_best_bid(&self) -> Option<&Order> {
        self.bids.first()
    }
    pub fn get_best_ask(&self) -> Option<&Order> {
        self.asks.first()
    }
    pub fn get_mid_price(&self) -> Option<Decimal> {
        self.get_best_ask().and_then(|ask| {
            self.get_best_bid()
                .map(|bid| (ask.price + bid.price) / Decimal::from_f64(2.0).unwrap())
        })
    }
    pub fn get_average_bid_price(&self, amount: Decimal) -> Option<Decimal> {
        if amount <= Decimal::ZERO {
            return None;
        }
        let mut total = Decimal::ZERO;
        let mut amount_left = amount;
        for bid in &self.bids {
            if amount_left > bid.size {
                total += bid.price * bid.size;
                amount_left -= bid.size;
            } else {
                total += bid.price * amount_left;
                amount_left = Decimal::ZERO;
                break;
            }
        }
        if amount_left > Decimal::ZERO {
            None
        } else {
            Some(total / amount)
        }
    }
    pub fn get_average_ask_price(&self, amount: Decimal) -> Option<Decimal> {
        if amount <= Decimal::ZERO {
            return None;
        }
        let mut total = Decimal::ZERO;
        let mut amount_left = amount;
        for ask in &self.asks {
            if amount_left > ask.size {
                total += ask.price * ask.size;
                amount_left -= ask.size;
            } else {
                total += ask.price * amount_left;
                amount_left = Decimal::ZERO;
                break;
            }
        }
        if amount_left > Decimal::ZERO {
            None
        } else {
            Some(total / amount)
        }
    }
    pub fn get_min_bid_price(&self, amount: Decimal) -> Option<Decimal> {
        if amount <= Decimal::ZERO {
            return None;
        }
        let mut amount_left = amount;
        for bid in &self.bids {
            if amount_left <= bid.size {
                return Some(bid.price);
            } else {
                amount_left -= bid.size;
            }
        }
        None
    }
    pub fn get_max_ask_price(&self, amount: Decimal) -> Option<Decimal> {
        if amount <= Decimal::ZERO {
            return None;
        }
        let mut amount_left = amount;
        for ask in &self.asks {
            if amount_left <= ask.size {
                return Some(ask.price);
            } else {
                amount_left -= ask.size;
            }
        }
        None
    }

    pub(crate) fn update(&mut self, update: OrderbookUpdate) {
        match update {
            OrderbookUpdate::Snapshot {
                timestamp_millis,
                bids,
                asks,
            } => {
                self.timestamp_millis = timestamp_millis;
                self.bids = bids;
                self.asks = asks;
            }
            OrderbookUpdate::OrdersUpdate {
                timestamp_millis,
                orders_update,
            } => {
                self.timestamp_millis = timestamp_millis;

                for order_update in orders_update {
                    let order = Order {
                        price: order_update.price,
                        size: order_update.size,
                    };
                    match order_update.side {
                        Side::Buy => match order_update.update_type {
                            OrderUpdateType::Added => {
                                self.add_bid(order);
                            }
                            OrderUpdateType::Changed => {
                                if order.size != Decimal::ZERO {
                                    self.insert_bid(order);
                                } else {
                                    self.bids.retain(|bid| bid.price != order.price);
                                }
                            }
                            OrderUpdateType::Removed => {
                                self.subtract_bid(order);
                            }
                        },
                        Side::Sell => match order_update.update_type {
                            OrderUpdateType::Added => {
                                self.add_ask(order);
                            }
                            OrderUpdateType::Changed => {
                                if order.size != Decimal::ZERO {
                                    self.insert_ask(order);
                                } else {
                                    self.asks.retain(|ask| ask.price != order.price);
                                }
                            }
                            OrderUpdateType::Removed => {
                                self.subtract_ask(order);
                            }
                        },
                    }
                }
            }
        }
    }
    pub fn insert_ask(&mut self, order: Order) {
        //insert ask at the right place
        let mut index = 0;
        for ask in &self.asks {
            match order.price.cmp(&ask.price) {
                std::cmp::Ordering::Less => break,
                std::cmp::Ordering::Equal => {
                    // remove previous order
                    self.asks.remove(index);
                    break;
                }
                std::cmp::Ordering::Greater => {}
            }

            index += 1;
        }
        self.asks.insert(index, order);
    }
    pub fn insert_bid(&mut self, order: Order) {
        //insert bid at the right place
        let mut index = 0;
        for bid in &self.bids {
            match order.price.cmp(&bid.price) {
                std::cmp::Ordering::Greater => break,
                std::cmp::Ordering::Equal => {
                    // remove previous order
                    self.bids.remove(index);
                    break;
                }
                std::cmp::Ordering::Less => {}
            }

            index += 1;
        }
        self.bids.insert(index, order);
    }
    pub fn add_ask(&mut self, order: Order) {
        //insert ask at the right place adding the order amount if the price is the same
        let mut index = 0;
        for ask in &self.asks {
            match order.price.cmp(&ask.price) {
                std::cmp::Ordering::Less => break,
                std::cmp::Ordering::Equal => {
                    // add to previous order
                    self.asks[index].size += order.size;
                    return;
                }
                std::cmp::Ordering::Greater => {}
            }

            index += 1;
        }
        self.asks.insert(index, order);
    }
    pub fn add_bid(&mut self, order: Order) {
        //insert bid at the right place adding the order amount if the price is the same
        let mut index = 0;
        for bid in &self.bids {
            match order.price.cmp(&bid.price) {
                std::cmp::Ordering::Greater => break,
                std::cmp::Ordering::Equal => {
                    // add to previous order
                    self.bids[index].size += order.size;
                    return;
                }
                std::cmp::Ordering::Less => {}
            }

            index += 1;
        }
        self.bids.insert(index, order);
    }
    pub fn subtract_ask(&mut self, order: Order) {
        //insert ask at the right place subtracting the order amount if the price is the same
        for (index, ask) in self.asks.iter().enumerate() {
            if ask.price == order.price {
                if ask.size > order.size {
                    // subtract from previous order
                    self.asks[index].size -= order.size;
                } else {
                    // remove previous order
                    self.asks.remove(index);
                }
                return;
            }
        }
    }
    pub fn subtract_bid(&mut self, order: Order) {
        //insert bid at the right place subtracting the order amount if the price is the same
        for (index, bid) in self.bids.iter().enumerate() {
            if bid.price == order.price {
                if bid.size > order.size {
                    // subtract from previous order
                    self.bids[index].size -= order.size;
                } else {
                    // remove previous order
                    self.bids.remove(index);
                }
                return;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn prepare_orderbook() -> Orderbook {
        Orderbook {
            timestamp_millis: 0,
            bids: vec![
                Order {
                    price: Decimal::from_f64(990.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(980.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(970.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
            ],
            asks: vec![
                Order {
                    price: Decimal::from_f64(1010.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1020.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1030.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
            ],
        }
    }

    #[test]
    fn test_get_best_bid() {
        let orderbook = prepare_orderbook();
        assert_eq!(
            orderbook.get_best_bid(),
            Some(&Order {
                price: Decimal::from_f64(990.0).unwrap(),
                size: Decimal::from_f64(10.0).unwrap(),
            })
        );
    }

    #[test]
    fn test_get_best_ask() {
        let orderbook = prepare_orderbook();
        assert_eq!(
            orderbook.get_best_ask(),
            Some(&Order {
                price: Decimal::from_f64(1010.0).unwrap(),
                size: Decimal::from_f64(10.0).unwrap(),
            })
        );
    }

    #[test]
    fn test_get_mean_price() {
        let orderbook = prepare_orderbook();
        assert_eq!(
            orderbook.get_mid_price(),
            Some(Decimal::from_f64(1000.0).unwrap())
        );
    }

    #[test]
    fn test_get_average_ask_price() {
        let orderbook = prepare_orderbook();
        assert_eq!(orderbook.get_average_ask_price(Decimal::ZERO), None);
        assert_eq!(
            orderbook.get_average_ask_price(Decimal::from_f64(5.0).unwrap()),
            Some(Decimal::from_f64(1010.0).unwrap())
        );
        assert_eq!(
            orderbook.get_average_ask_price(Decimal::from_f64(10.0).unwrap()),
            Some(Decimal::from_f64(1010.0).unwrap())
        );
        assert_eq!(
            orderbook.get_average_ask_price(Decimal::from_f64(16.0).unwrap()),
            Some(Decimal::from_f64(1013.75).unwrap())
        );
        assert_eq!(
            orderbook.get_average_ask_price(Decimal::from_f64(20.0).unwrap()),
            Some(Decimal::from_f64(1015.0).unwrap())
        );
        assert_eq!(
            orderbook.get_average_ask_price(Decimal::from_f64(30.0).unwrap()),
            Some(Decimal::from_f64(1020.0).unwrap())
        );
        assert_eq!(
            orderbook.get_average_ask_price(Decimal::from_f64(100.0).unwrap()),
            None
        );
    }

    #[test]
    fn test_get_average_bid_price() {
        let orderbook = prepare_orderbook();
        assert_eq!(orderbook.get_average_bid_price(Decimal::ZERO), None);
        assert_eq!(
            orderbook.get_average_bid_price(Decimal::from_f64(5.0).unwrap()),
            Some(Decimal::from_f64(990.0).unwrap())
        );
        assert_eq!(
            orderbook.get_average_bid_price(Decimal::from_f64(10.0).unwrap()),
            Some(Decimal::from_f64(990.0).unwrap())
        );
        assert_eq!(
            orderbook.get_average_bid_price(Decimal::from_f64(16.0).unwrap()),
            Some(Decimal::from_f64(986.25).unwrap())
        );
        assert_eq!(
            orderbook.get_average_bid_price(Decimal::from_f64(20.0).unwrap()),
            Some(Decimal::from_f64(985.0).unwrap())
        );
        assert_eq!(
            orderbook.get_average_bid_price(Decimal::from_f64(30.0).unwrap()),
            Some(Decimal::from_f64(980.0).unwrap())
        );
        assert_eq!(
            orderbook.get_average_bid_price(Decimal::from_f64(100.0).unwrap()),
            None
        );
    }

    #[test]
    fn test_get_max_ask_price() {
        let orderbook = prepare_orderbook();
        assert_eq!(orderbook.get_max_ask_price(Decimal::ZERO), None);
        assert_eq!(
            orderbook.get_max_ask_price(Decimal::from_f64(5.0).unwrap()),
            Some(Decimal::from_f64(1010.0).unwrap())
        );
        assert_eq!(
            orderbook.get_max_ask_price(Decimal::from_f64(10.0).unwrap()),
            Some(Decimal::from_f64(1010.0).unwrap())
        );
        assert_eq!(
            orderbook.get_max_ask_price(Decimal::from_f64(16.0).unwrap()),
            Some(Decimal::from_f64(1020.0).unwrap())
        );
        assert_eq!(
            orderbook.get_max_ask_price(Decimal::from_f64(20.0).unwrap()),
            Some(Decimal::from_f64(1020.0).unwrap())
        );
        assert_eq!(
            orderbook.get_max_ask_price(Decimal::from_f64(30.0).unwrap()),
            Some(Decimal::from_f64(1030.0).unwrap())
        );
        assert_eq!(
            orderbook.get_max_ask_price(Decimal::from_f64(100.0).unwrap()),
            None
        );
    }

    #[test]
    fn test_get_min_bid_price() {
        let orderbook = prepare_orderbook();
        assert_eq!(orderbook.get_min_bid_price(Decimal::ZERO), None);
        assert_eq!(
            orderbook.get_min_bid_price(Decimal::from_f64(5.0).unwrap()),
            Some(Decimal::from_f64(990.0).unwrap())
        );
        assert_eq!(
            orderbook.get_min_bid_price(Decimal::from_f64(10.0).unwrap()),
            Some(Decimal::from_f64(990.0).unwrap())
        );
        assert_eq!(
            orderbook.get_min_bid_price(Decimal::from_f64(16.0).unwrap()),
            Some(Decimal::from_f64(980.0).unwrap())
        );
        assert_eq!(
            orderbook.get_min_bid_price(Decimal::from_f64(20.0).unwrap()),
            Some(Decimal::from_f64(980.0).unwrap())
        );
        assert_eq!(
            orderbook.get_min_bid_price(Decimal::from_f64(30.0).unwrap()),
            Some(Decimal::from_f64(970.0).unwrap())
        );
        assert_eq!(
            orderbook.get_min_bid_price(Decimal::from_f64(100.0).unwrap()),
            None
        );
    }

    #[test]
    fn test_insert_ask() {
        let mut orderbook = prepare_orderbook();
        orderbook.insert_ask(Order {
            price: Decimal::from_f64(1005.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        orderbook.insert_ask(Order {
            price: Decimal::from_f64(1015.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        orderbook.insert_ask(Order {
            price: Decimal::from_f64(1040.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        assert_eq!(
            orderbook.asks,
            vec![
                Order {
                    price: Decimal::from_f64(1005.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1010.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1015.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1020.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1030.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1040.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
            ]
        );
        orderbook.insert_ask(Order {
            price: Decimal::from_f64(1040.0).unwrap(),
            size: Decimal::from_f64(20.0).unwrap(),
        });
        assert_eq!(
            orderbook.asks,
            vec![
                Order {
                    price: Decimal::from_f64(1005.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1010.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1015.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1020.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1030.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1040.0).unwrap(),
                    size: Decimal::from_f64(20.0).unwrap(),
                },
            ]
        );
    }

    #[test]
    fn test_insert_bid() {
        let mut orderbook = prepare_orderbook();
        orderbook.insert_bid(Order {
            price: Decimal::from_f64(995.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        orderbook.insert_bid(Order {
            price: Decimal::from_f64(985.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        orderbook.insert_bid(Order {
            price: Decimal::from_f64(960.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        assert_eq!(
            orderbook.bids,
            vec![
                Order {
                    price: Decimal::from_f64(995.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(990.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(985.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(980.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(970.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(960.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
            ]
        );
        orderbook.insert_bid(Order {
            price: Decimal::from_f64(960.0).unwrap(),
            size: Decimal::from_f64(20.0).unwrap(),
        });
        assert_eq!(
            orderbook.bids,
            vec![
                Order {
                    price: Decimal::from_f64(995.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(990.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(985.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(980.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(970.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(960.0).unwrap(),
                    size: Decimal::from_f64(20.0).unwrap(),
                },
            ]
        );
    }

    #[test]
    fn test_add_ask() {
        let mut orderbook = prepare_orderbook();
        orderbook.add_ask(Order {
            price: Decimal::from_f64(1005.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        orderbook.add_ask(Order {
            price: Decimal::from_f64(1010.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        orderbook.add_ask(Order {
            price: Decimal::from_f64(1030.0).unwrap(),
            size: Decimal::from_f64(20.0).unwrap(),
        });
        assert_eq!(
            orderbook.asks,
            vec![
                Order {
                    price: Decimal::from_f64(1005.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1010.0).unwrap(),
                    size: Decimal::from_f64(20.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1020.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1030.0).unwrap(),
                    size: Decimal::from_f64(30.0).unwrap(),
                },
            ]
        );
    }

    #[test]
    fn test_add_bid() {
        let mut orderbook = prepare_orderbook();
        orderbook.add_bid(Order {
            price: Decimal::from_f64(995.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        orderbook.add_bid(Order {
            price: Decimal::from_f64(990.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        orderbook.add_bid(Order {
            price: Decimal::from_f64(970.0).unwrap(),
            size: Decimal::from_f64(20.0).unwrap(),
        });
        assert_eq!(
            orderbook.bids,
            vec![
                Order {
                    price: Decimal::from_f64(995.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(990.0).unwrap(),
                    size: Decimal::from_f64(20.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(980.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(970.0).unwrap(),
                    size: Decimal::from_f64(30.0).unwrap(),
                },
            ]
        );
    }

    #[test]
    fn test_subtract_ask() {
        let mut orderbook = prepare_orderbook();
        orderbook.subtract_ask(Order {
            price: Decimal::from_f64(1010.0).unwrap(),
            size: Decimal::from_f64(5.0).unwrap(),
        });
        orderbook.subtract_ask(Order {
            price: Decimal::from_f64(1030.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        assert_eq!(
            orderbook.asks,
            vec![
                Order {
                    price: Decimal::from_f64(1010.0).unwrap(),
                    size: Decimal::from_f64(5.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1020.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
            ]
        );
    }

    #[test]
    fn test_subtract_bid() {
        let mut orderbook = prepare_orderbook();
        orderbook.subtract_bid(Order {
            price: Decimal::from_f64(990.0).unwrap(),
            size: Decimal::from_f64(5.0).unwrap(),
        });
        orderbook.subtract_bid(Order {
            price: Decimal::from_f64(970.0).unwrap(),
            size: Decimal::from_f64(10.0).unwrap(),
        });
        assert_eq!(
            orderbook.bids,
            vec![
                Order {
                    price: Decimal::from_f64(990.0).unwrap(),
                    size: Decimal::from_f64(5.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(980.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
            ]
        );
    }

    #[test]
    fn test_update_orderbook() {
        let mut orderbook = prepare_orderbook();
        let orders_update = vec![
            OrderUpdate {
                price: Decimal::from_f64(995.0).unwrap(),
                size: Decimal::from_f64(10.0).unwrap(),
                side: Side::Buy,
                update_type: OrderUpdateType::Added,
            },
            OrderUpdate {
                price: Decimal::from_f64(990.0).unwrap(),
                size: Decimal::from_f64(20.0).unwrap(),
                side: Side::Buy,
                update_type: OrderUpdateType::Changed,
            },
            OrderUpdate {
                price: Decimal::from_f64(980.0).unwrap(),
                size: Decimal::from_f64(10.0).unwrap(),
                side: Side::Buy,
                update_type: OrderUpdateType::Removed,
            },
            OrderUpdate {
                price: Decimal::from_f64(1005.0).unwrap(),
                size: Decimal::from_f64(10.0).unwrap(),
                side: Side::Sell,
                update_type: OrderUpdateType::Added,
            },
            OrderUpdate {
                price: Decimal::from_f64(1010.0).unwrap(),
                size: Decimal::from_f64(20.0).unwrap(),
                side: Side::Sell,
                update_type: OrderUpdateType::Changed,
            },
            OrderUpdate {
                price: Decimal::from_f64(1020.0).unwrap(),
                size: Decimal::from_f64(10.0).unwrap(),
                side: Side::Sell,
                update_type: OrderUpdateType::Removed,
            },
        ];

        orderbook.update(OrderbookUpdate::OrdersUpdate {
            timestamp_millis: 1,
            orders_update,
        });

        assert_eq!(
            orderbook.bids,
            vec![
                Order {
                    price: Decimal::from_f64(995.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(990.0).unwrap(),
                    size: Decimal::from_f64(20.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(970.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
            ]
        );
        assert_eq!(
            orderbook.asks,
            vec![
                Order {
                    price: Decimal::from_f64(1005.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1010.0).unwrap(),
                    size: Decimal::from_f64(20.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1030.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
            ]
        );

        let bids_update = vec![
            Order {
                price: Decimal::from_f64(995.0).unwrap(),
                size: Decimal::from_f64(10.0).unwrap(),
            },
            Order {
                price: Decimal::from_f64(990.0).unwrap(),
                size: Decimal::from_f64(20.0).unwrap(),
            },
        ];

        let asks_update = vec![
            Order {
                price: Decimal::from_f64(1005.0).unwrap(),
                size: Decimal::from_f64(10.0).unwrap(),
            },
            Order {
                price: Decimal::from_f64(1010.0).unwrap(),
                size: Decimal::from_f64(20.0).unwrap(),
            },
        ];

        orderbook.update(OrderbookUpdate::Snapshot {
            timestamp_millis: 2,
            bids: bids_update,
            asks: asks_update,
        });

        assert_eq!(
            orderbook.bids,
            vec![
                Order {
                    price: Decimal::from_f64(995.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(990.0).unwrap(),
                    size: Decimal::from_f64(20.0).unwrap(),
                },
            ]
        );
        assert_eq!(
            orderbook.asks,
            vec![
                Order {
                    price: Decimal::from_f64(1005.0).unwrap(),
                    size: Decimal::from_f64(10.0).unwrap(),
                },
                Order {
                    price: Decimal::from_f64(1010.0).unwrap(),
                    size: Decimal::from_f64(20.0).unwrap(),
                },
            ]
        );
    }
}
