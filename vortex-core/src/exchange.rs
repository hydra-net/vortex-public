use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

use anyhow::Error;
use async_trait::async_trait;
use futures::StreamExt;
use rust_decimal::Decimal;
use tokio::{
    sync::{Notify, RwLock},
    task::JoinHandle,
};
use tracing::{debug, error, info};

use crate::types::{
    Balance, BalanceUpdateSink, BalanceUpdateStream, Orderbook, OrderbookUpdateStream, Side,
    TradeOrder, TradeOrderUpdateStream, TradeOrders,
};
use crate::{
    api::{
        BalanceFetcher, CreateOrderError, InfoFetcher, OrderCreator, OrderFetcher, OrderbookFetcher,
    },
    types::Info,
};

#[async_trait]
pub trait ExchangeAPI:
    BalanceFetcher
    + OrderbookFetcher
    + InfoFetcher
    + OrderFetcher
    + OrderCreator
    + Send
    + Sync
    + 'static
{
    fn name(&self) -> String;

    async fn wait_sync(&self);
}

pub struct ExchangeClient {
    name: String,
    client: Arc<dyn ExchangeAPI>,
    balance_update_sinks: Arc<Mutex<HashMap<String, Vec<BalanceUpdateSink>>>>, // currency -> sinks
    last_currency_trade_timestamp_ms: Arc<RwLock<HashMap<String, u64>>>, // currency -> timestamp_ms
    sync_task_handle: JoinHandle<()>,
    is_synced: Arc<AtomicBool>,
    sync_notifier: Arc<Notify>,
}

impl Drop for ExchangeClient {
    fn drop(&mut self) {
        self.sync_task_handle.abort();
    }
}

impl ExchangeClient {
    pub async fn new(client: impl ExchangeAPI) -> Self {
        let client = Arc::new(client);
        let name = client.name();

        let balance_update_sinks = Arc::new(Mutex::new(HashMap::new()));
        let last_currency_trade_timestamp_ms = Arc::new(RwLock::new(HashMap::new()));
        let is_synced = Arc::new(AtomicBool::new(false));
        let sync_notifier = Arc::new(Notify::new());

        let sync_task_handle = start_sync(
            client.clone(),
            balance_update_sinks.clone(),
            is_synced.clone(),
            sync_notifier.clone(),
        );

        ExchangeClient {
            name: name.clone(),
            client: client.clone(),
            balance_update_sinks,
            last_currency_trade_timestamp_ms,
            sync_task_handle,
            is_synced,
            sync_notifier,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn wait_sync(&self) {
        if !self.is_synced.load(std::sync::atomic::Ordering::Acquire) {
            self.sync_notifier.notified().await;
        }
    }

    pub async fn get_balances(&self) -> Result<HashMap<String, Balance>, Error> {
        self.client.get_balances().await
    }

    pub async fn get_market_info(&self, pair: &str) -> Result<Info, Error> {
        self.client.get_market_info(pair).await
    }

    pub async fn get_orderbook(&self, pair: &str) -> Result<Orderbook, Error> {
        self.client.get_orderbook(pair).await
    }

    pub async fn subscribe_orderbook(&self, pair: &str) -> Result<OrderbookUpdateStream, Error> {
        self.client.subscribe_orderbook(pair).await
    }

    pub async fn get_orders_pending(&self, pair: &str) -> Result<TradeOrders, Error> {
        self.client.get_orders_pending(pair).await
    }

    pub async fn subscribe_orders_pending(
        &self,
        pair: &str,
    ) -> Result<TradeOrderUpdateStream, Error> {
        self.client.subscribe_orders_pending(pair).await
    }

    pub async fn create_limit_order(
        &self,
        pair: &str,
        side: Side,
        amount: Decimal,
        price: Decimal,
    ) -> Result<TradeOrder, CreateOrderError> {
        let mut last_currency_trade_timestamp_ms =
            self.last_currency_trade_timestamp_ms.write().await;

        let (base, quote) = pair
            .split_once('_')
            .ok_or(CreateOrderError::Other(Error::msg(
                "Invalid pair format, expected base_quote",
            )))?;

        let order = self
            .client
            .create_limit_order(pair, side, amount, price)
            .await?;

        last_currency_trade_timestamp_ms.insert(base.to_string(), order.timestamp_millis);
        last_currency_trade_timestamp_ms.insert(quote.to_string(), order.timestamp_millis);

        Ok(order)
    }

    pub async fn create_market_order(
        &self,
        pair: &str,
        side: Side,
        amount: Decimal,
    ) -> Result<TradeOrder, CreateOrderError> {
        let mut last_currency_trade_timestamp_ms =
            self.last_currency_trade_timestamp_ms.write().await;

        let (base, quote) = pair
            .split_once('_')
            .ok_or(CreateOrderError::Other(Error::msg(
                "Invalid pair format, expected base_quote",
            )))?;

        let order = self.client.create_market_order(pair, side, amount).await?;

        last_currency_trade_timestamp_ms.insert(base.to_string(), order.timestamp_millis);
        last_currency_trade_timestamp_ms.insert(quote.to_string(), order.timestamp_millis);

        Ok(order)
    }

    pub async fn create_hold_market_order(
        &self,
        pair: &str,
        side: Side,
        amount: Decimal,
    ) -> Result<TradeOrder, CreateOrderError> {
        let mut last_currency_trade_timestamp_ms =
            self.last_currency_trade_timestamp_ms.write().await;

        let (base, quote) = pair
            .split_once('_')
            .ok_or(CreateOrderError::Other(Error::msg(
                "Invalid pair format, expected base_quote",
            )))?;
        let order = self
            .client
            .create_hold_market_order(pair, side, amount)
            .await?;

        last_currency_trade_timestamp_ms.insert(base.to_string(), order.timestamp_millis);
        last_currency_trade_timestamp_ms.insert(quote.to_string(), order.timestamp_millis);

        Ok(order)
    }

    pub async fn cancel_order(&self, pair: &str, id: &str) -> Result<(), CreateOrderError> {
        self.client.cancel_order(pair, id).await
    }

    pub async fn cancel_all_orders(&self, pair: &str) -> Result<(), CreateOrderError> {
        self.client.cancel_all_orders(pair).await
    }

    pub fn balance_update_stream(&self, currency: &str) -> BalanceUpdateStream {
        let (balance_update_sink, balance_update_stream) = futures::channel::mpsc::unbounded();
        self.balance_update_sinks
            .lock()
            .unwrap()
            .entry(currency.to_string())
            .or_default()
            .push(balance_update_sink);

        balance_update_stream.boxed()
    }

    pub(crate) async fn update_last_trade_timestamp_millis(
        &self,
        currency: &str,
        timestamp_millis: u64,
    ) {
        let mut last_currency_trade_timestamp_ms =
            self.last_currency_trade_timestamp_ms.write().await;
        last_currency_trade_timestamp_ms
            .entry(currency.to_string())
            .and_modify(|t| *t = (*t).max(timestamp_millis))
            .or_insert(timestamp_millis);
    }

    /// Returns true if there is an order in progress that involves the given currency.
    pub async fn last_trade_timestamp_millis(&self, currency: &str) -> Option<u64> {
        self.last_currency_trade_timestamp_ms
            .read()
            .await
            .get(currency)
            .copied()
    }
}

fn start_sync(
    exchange: Arc<dyn ExchangeAPI>,
    balance_update_sinks: Arc<Mutex<HashMap<String, Vec<BalanceUpdateSink>>>>,
    is_synced: Arc<AtomicBool>,
    sync_notifier: Arc<Notify>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        debug!(
            "Spawning balances sync task for {} exchange",
            exchange.name()
        );

        loop {
            is_synced.store(false, std::sync::atomic::Ordering::Release);

            exchange.wait_sync().await;

            let mut balance_update_stream = match exchange.subscribe_balances().await {
                Ok(stream) => stream,
                Err(e) => {
                    error!(
                        "Failed to subscribe to {} balances updates: {:?}",
                        exchange.name(),
                        e
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            info!("{} exchange is ready!", exchange.name());

            is_synced.store(true, std::sync::atomic::Ordering::Release);
            sync_notifier.notify_waiters();

            while let Some(balance_update) = balance_update_stream.next().await {
                let mut sinks = balance_update_sinks.lock().unwrap();
                sinks
                    .entry(balance_update.currency.clone())
                    .or_default()
                    .retain(|sink| {
                        if sink.unbounded_send(balance_update.clone()).is_err() {
                            sink.close_channel();
                            false
                        } else {
                            true
                        }
                    });
            }

            error!("{} exchange balances update stream closed", exchange.name());

            for sinks in balance_update_sinks.lock().unwrap().values_mut() {
                for sink in sinks.drain(..) {
                    sink.close_channel();
                }
            }
        }
    })
}
