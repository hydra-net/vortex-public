use std::sync::{atomic::AtomicBool, Arc, Mutex, RwLock};

use anyhow::Error;
use futures::StreamExt;
use rust_decimal::Decimal;
use tokio::{sync::Notify, task::JoinHandle};
use tracing::{debug, error, info};

use crate::{
    api::CreateOrderError,
    exchange::ExchangeClient,
    types::{
        Balance, BalanceUpdateStream, Info, MarketUpdate, MarketUpdateSink, MarketUpdateStream,
        Orderbook, OrderbookUpdateStream, Side, TradeOrder, TradeOrderUpdate,
        TradeOrderUpdateStream, TradeOrders,
    },
};

pub struct Market {
    exchange: Arc<ExchangeClient>,
    info: Info,
    orderbook: Arc<RwLock<Orderbook>>,
    pending_orders: Arc<RwLock<TradeOrders>>,
    completed_orders: Arc<RwLock<TradeOrders>>,
    base_balance: Arc<RwLock<Balance>>,
    quote_balance: Arc<RwLock<Balance>>,
    update_sinks: Arc<Mutex<Vec<MarketUpdateSink>>>,
    sync_task_handle: JoinHandle<()>,
    is_synced: Arc<AtomicBool>,
    sync_notifier: Arc<Notify>,
}

impl Drop for Market {
    fn drop(&mut self) {
        self.sync_task_handle.abort();
    }
}

impl Market {
    pub async fn new(
        exchange: Arc<ExchangeClient>,
        pair: &str,
        cancel_orders: bool,
    ) -> Result<Self, Error> {
        let info = exchange.get_market_info(pair).await?;

        if cancel_orders {
            exchange.cancel_all_orders(pair).await?;
        }

        let orderbook = Arc::new(RwLock::new(Orderbook::default()));
        let pending_orders = Arc::new(RwLock::new(TradeOrders::default()));
        let completed_orders = Arc::new(RwLock::new(TradeOrders::default()));
        let base_balance = Arc::new(RwLock::new(Balance::default()));
        let quote_balance = Arc::new(RwLock::new(Balance::default()));
        let update_sinks = Arc::new(Mutex::new(Vec::new()));
        let is_synced = Arc::new(AtomicBool::new(false));
        let sync_notifier = Arc::new(Notify::new());

        let sync_task_handle = start_sync(
            exchange.clone(),
            pair.to_string(),
            orderbook.clone(),
            pending_orders.clone(),
            completed_orders.clone(),
            base_balance.clone(),
            quote_balance.clone(),
            update_sinks.clone(),
            is_synced.clone(),
            sync_notifier.clone(),
        )
        .await;

        Ok(Market {
            exchange,
            info,
            orderbook,
            pending_orders,
            completed_orders,
            base_balance,
            quote_balance,
            update_sinks,
            sync_task_handle,
            is_synced,
            sync_notifier,
        })
    }

    pub fn exchange(&self) -> &str {
        self.exchange.name()
    }

    pub async fn wait_sync(&self) {
        if !self.is_synced.load(std::sync::atomic::Ordering::Acquire) {
            self.sync_notifier.notified().await;
        }
    }

    pub fn pair(&self) -> &str {
        &self.info.pair
    }

    pub fn base(&self) -> &str {
        &self.info.base
    }

    pub fn quote(&self) -> &str {
        &self.info.quote
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn orderbook(&self) -> Orderbook {
        self.orderbook.read().unwrap().clone()
    }

    pub fn pending_orders(&self) -> TradeOrders {
        self.pending_orders.read().unwrap().clone()
    }

    pub fn completed_orders(&self) -> TradeOrders {
        self.completed_orders.read().unwrap().clone()
    }

    pub fn base_balance(&self) -> Balance {
        self.base_balance.read().unwrap().clone()
    }

    pub fn quote_balance(&self) -> Balance {
        self.quote_balance.read().unwrap().clone()
    }

    pub async fn create_limit_order(
        &self,
        side: Side,
        amount: Decimal,
        price: Decimal,
    ) -> Result<TradeOrder, CreateOrderError> {
        let price_precision = self.info.price_precision;
        let amount_precision = self.info.amount_precision;

        let amount = amount.round_dp(amount_precision as u32);
        let price = price.round_dp(price_precision as u32);

        let order = self
            .exchange
            .create_limit_order(&self.info.pair, side, amount, price)
            .await?;

        self.pending_orders
            .write()
            .unwrap()
            .insert_order(order.clone());

        Ok(order)
    }

    pub async fn create_market_order(
        &self,
        side: Side,
        amount: Decimal,
    ) -> Result<TradeOrder, CreateOrderError> {
        let amount_precision = self.info.amount_precision;

        let amount = amount.round_dp(amount_precision as u32);

        let order = self
            .exchange
            .create_market_order(&self.info.pair, side, amount)
            .await?;

        self.pending_orders
            .write()
            .unwrap()
            .insert_order(order.clone());

        Ok(order)
    }

    pub async fn create_hold_market_order(
        &self,
        pair: &str,
        side: Side,
        amount: Decimal,
    ) -> Result<TradeOrder, CreateOrderError> {
        let amount_precision = self.info.amount_precision;

        let amount = amount.round_dp(amount_precision as u32);

        let order = self
            .exchange
            .create_hold_market_order(pair, side, amount)
            .await?;

        self.pending_orders
            .write()
            .unwrap()
            .insert_order(order.clone());

        Ok(order)
    }

    pub async fn cancel_order(&self, id: &str) -> Result<(), CreateOrderError> {
        self.exchange.cancel_order(&self.info.pair, id).await
    }

    pub async fn cancel_all_orders(&self) -> Result<(), CreateOrderError> {
        self.exchange.cancel_all_orders(&self.info.pair).await
    }

    /// Returns the latest trade timestamp that involves the base or quote currency.
    /// This is useful to check if the current balances of the currencies are up to date and we can
    /// safely place orders without risking insufficient funds error.
    pub async fn last_trade_timestamp_millis(&self) -> u64 {
        let base_last_trade_timestamp_ms = self
            .exchange
            .last_trade_timestamp_millis(self.base())
            .await
            .unwrap_or_default();
        let quote_last_trade_timestamp_ms = self
            .exchange
            .last_trade_timestamp_millis(self.quote())
            .await
            .unwrap_or_default();

        base_last_trade_timestamp_ms.max(quote_last_trade_timestamp_ms)
    }

    pub fn update_stream(&self) -> MarketUpdateStream {
        let (update_sink, update_stream) = futures::channel::mpsc::unbounded();

        self.update_sinks.lock().unwrap().push(update_sink);

        update_stream.boxed()
    }
}

#[allow(clippy::too_many_arguments)]
async fn start_sync(
    exchange: Arc<ExchangeClient>,
    pair: String,
    orderbook: Arc<RwLock<Orderbook>>,
    pending_orders: Arc<RwLock<TradeOrders>>,
    completed_orders: Arc<RwLock<TradeOrders>>,
    base_balance: Arc<RwLock<Balance>>,
    quote_balance: Arc<RwLock<Balance>>,
    update_sinks: Arc<Mutex<Vec<MarketUpdateSink>>>,
    is_synced: Arc<AtomicBool>,
    sync_notifier: Arc<Notify>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        debug!(
            "Spawning {} sync task for {} exchange",
            pair,
            exchange.name()
        );

        loop {
            is_synced.store(false, std::sync::atomic::Ordering::Release);

            exchange.wait_sync().await;

            let (base, quote) = pair.split_once('_').unwrap();

            match exchange.get_balances().await {
                Ok(balances) => {
                    *base_balance.write().unwrap() =
                        balances.get(base).cloned().unwrap_or_default();
                    *quote_balance.write().unwrap() =
                        balances.get(quote).cloned().unwrap_or_default();
                }
                Err(e) => {
                    error!(
                        "Failed to get {} exchange balances: {:?}",
                        exchange.name(),
                        e
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            match exchange.get_orders_pending(&pair).await {
                Ok(orders) => *pending_orders.write().unwrap() = orders,
                Err(e) => {
                    error!(
                        "Failed to get {} exchange pending orders: {:?}",
                        exchange.name(),
                        e
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            match exchange.get_orderbook(&pair).await {
                Ok(ob) => *orderbook.write().unwrap() = ob,
                Err(e) => {
                    error!(
                        "Failed to get {} exchange orderbook: {:?}",
                        exchange.name(),
                        e
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            let base_balance_update_stream = exchange.balance_update_stream(base);
            let quote_balance_update_stream = exchange.balance_update_stream(quote);

            let trade_order_update_stream = match exchange.subscribe_orders_pending(&pair).await {
                Ok(stream) => stream,
                Err(e) => {
                    error!(
                        "Failed to subscribe to {} exchange pending orders updates: {:?}",
                        exchange.name(),
                        e
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            let orderbook_update_stream = match exchange.subscribe_orderbook(&pair).await {
                Ok(stream) => stream,
                Err(e) => {
                    error!(
                        "Failed to subscribe to {} exchange orderbook updates: {:?}",
                        exchange.name(),
                        e
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            info!("{} market on {} exchange is ready!", pair, exchange.name());

            is_synced.store(true, std::sync::atomic::Ordering::Release);
            sync_notifier.notify_waiters();

            sync_update_streams(
                exchange.clone(),
                pair.clone(),
                orderbook.clone(),
                pending_orders.clone(),
                completed_orders.clone(),
                base_balance.clone(),
                quote_balance.clone(),
                orderbook_update_stream,
                trade_order_update_stream,
                base_balance_update_stream,
                quote_balance_update_stream,
                update_sinks.clone(),
            )
            .await;

            // close all update streams to notify the strategy tasks that the market is not synced
            for sink in update_sinks.lock().unwrap().drain(..) {
                sink.close_channel();
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
async fn sync_update_streams(
    exchange: Arc<ExchangeClient>,
    pair: String,
    orderbook: Arc<RwLock<Orderbook>>,
    pending_orders: Arc<RwLock<TradeOrders>>,
    completed_orders: Arc<RwLock<TradeOrders>>,
    base_balance: Arc<RwLock<Balance>>,
    quote_balance: Arc<RwLock<Balance>>,
    mut orderbook_update_stream: OrderbookUpdateStream,
    mut trade_order_update_stream: TradeOrderUpdateStream,
    mut base_balance_update_stream: BalanceUpdateStream,
    mut quote_balance_update_stream: BalanceUpdateStream,
    update_sinks: Arc<Mutex<Vec<MarketUpdateSink>>>,
) {
    loop {
        let market_update = tokio::select! {
            orderbook_update = orderbook_update_stream.next() => {
                if let Some(orderbook_update) = orderbook_update {
                    orderbook.write().unwrap().update(orderbook_update.clone());

                    MarketUpdate::Orderbook(orderbook_update)
                } else {
                    error!(
                        "{} exchange orderbook update stream closed for {} pair",
                        exchange.name(),
                        pair
                    );

                    break;
                }
            },
            trade_order_update = trade_order_update_stream.next() => {
                if let Some(trade_order_update) = trade_order_update {
                    match trade_order_update.clone() {
                        TradeOrderUpdate::Added(order) => {
                            pending_orders.write().unwrap().insert_order(order);
                        }
                        TradeOrderUpdate::Changed(order) => {
                            pending_orders.write().unwrap().update_order(order);
                        }
                        TradeOrderUpdate::Completed { order_id, .. } => {
                            let order = pending_orders
                                .write()
                                .unwrap()
                                .remove_order_by_id(&order_id);

                            if let Some(mut order) = order {
                                order.executed = order.amount;
                                order.left = Decimal::ZERO;

                                let mut completed_orders = completed_orders.write().unwrap();
                                completed_orders.push_order(order);

                                // remove completed orders over 100 to avoid memory leak
                                if completed_orders.buys.len() > 100 {
                                    completed_orders.buys.remove(0);
                                } else if completed_orders.sells.len() > 100 {
                                    completed_orders.sells.remove(0);
                                }
                            }
                        }
                        TradeOrderUpdate::Canceled { order_id, .. } => {
                            let order = pending_orders
                                .write()
                                .unwrap()
                                .remove_order_by_id(&order_id);

                            if let Some(order) = order {
                                if order.executed > Decimal::ZERO {
                                    completed_orders.write().unwrap().push_order(order);
                                }
                            }
                        }
                    }

                    let (base, quote) = pair.split_once('_').unwrap();
                    exchange
                        .update_last_trade_timestamp_millis(base, trade_order_update.timestamp_millis())
                        .await;
                    exchange
                        .update_last_trade_timestamp_millis(quote, trade_order_update.timestamp_millis())
                        .await;

                    MarketUpdate::TradeOrder(trade_order_update)
                } else {
                    error!(
                        "{} exchange trade orders update stream closed for {} pair",
                        exchange.name(),
                        pair
                    );

                    break;
                }
            },
            base_balance_update = base_balance_update_stream.next() => {
                if let Some(base_balance_update) = base_balance_update {
                    *base_balance.write().unwrap() = base_balance_update.balance.clone();

                    MarketUpdate::Balance(base_balance_update)
                } else {
                    error!(
                        "{} exchange base balance update stream closed for {} pair",
                        exchange.name(),
                        pair
                    );

                    break;
                }
            },
            quote_balance_update = quote_balance_update_stream.next() => {
                if let Some(quote_balance_update) = quote_balance_update {
                    *quote_balance.write().unwrap() = quote_balance_update.balance.clone();

                    MarketUpdate::Balance(quote_balance_update)
                } else {
                    error!(
                        "{} exchange quote balance update stream closed for {} pair",
                        exchange.name(),
                        pair
                    );

                    break;
                }
            },
        };

        let mut sinks = update_sinks.lock().unwrap();
        sinks.retain(|sink| {
            if sink.unbounded_send(market_update.clone()).is_err() {
                sink.close_channel();
                false
            } else {
                true
            }
        });
    }
}
