//#![feature(never_type)]

use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    str::FromStr,
    sync::{atomic::AtomicBool, Arc},
    time::SystemTime,
};

use anyhow::Error;
use async_trait::async_trait;
use futures::{channel::mpsc::UnboundedSender, lock::Mutex, StreamExt};
use lnd_client::LndClient;
use protos::order_update;
use rust_decimal::{prelude::Zero, Decimal};
use tokio::{sync::Notify, task::JoinHandle};
use tonic::transport::Endpoint;
use tracing::{debug, error};
use utils::proto_order_to_trade_order;
use vortex_core::{
    exchange::ExchangeAPI,
    types::{OrderUpdate, OrderUpdateType, OrderbookUpdate, Side, TradeOrderUpdate},
};

mod balance;
mod config;
mod connext;
mod info;
mod orderbook;
mod orders;
mod trades;
mod utils;
mod protos {
    tonic::include_proto!("lssdrpc");
}

pub use self::config::{Asset, HydranetConfig};
use self::connext::ConnextClient;
use self::protos::{
    currencies_client::CurrenciesClient,
    lnd_configuration::{Macaroon, TlsCert},
    orders_client::OrdersClient,
    trading_pairs_client::TradingPairsClient,
};

type OrderbookUpdateSinks = HashMap<String, Vec<UnboundedSender<OrderbookUpdate>>>; // pair -> sinks
type TradeUpdateSinks = HashMap<String, Vec<UnboundedSender<TradeOrderUpdate>>>; // pair -> sinks

#[derive(Debug, Hash, Eq, PartialEq, Clone)]
struct PairOrderId {
    pair: String,
    order_id: String
}
pub struct Hydranet {
    orders_client: Arc<Mutex<OrdersClient<tonic::transport::Channel>>>,
    lnd_client: Mutex<LndClient>,
    connext_client: ConnextClient,
    config: HydranetConfig,
    stream_subscriptions_handle: JoinHandle<()>,
    orderbook_update_sinks: Arc<Mutex<OrderbookUpdateSinks>>,
    trade_update_sinks: Arc<Mutex<TradeUpdateSinks>>,
    is_synced: Arc<AtomicBool>,
    sync_notifier: Arc<Notify>,
    order_id_map: Arc<Mutex<HashMap<PairOrderId,String>>>
}

#[async_trait]
impl ExchangeAPI for Hydranet {
    fn name(&self) -> String {
        "Hydranet".to_string()
    }

    async fn wait_sync(&self) {
        if !self.is_synced.load(std::sync::atomic::Ordering::Acquire) {
            self.sync_notifier.notified().await;
        }
    }
}

impl Hydranet {
    pub async fn new(config: HydranetConfig) -> Result<Hydranet, Error> {
        let lssd_endpoint = Endpoint::from_str(&config.lssd_url)?;

        let mut currencies_client = CurrenciesClient::connect(lssd_endpoint.clone()).await?;
        let mut trading_pairs_client = TradingPairsClient::connect(lssd_endpoint.clone()).await?;
        let mut orders_client = OrdersClient::connect(lssd_endpoint).await?;

        let lnd_client = LndClient::new(
            &config.lnd_url,
            &config.lnd_cert_dir,
            &config.lnd_macaroon_dir,
        )
        .await?;
        let connext_client = ConnextClient::new(&config.connext_url);

        let mut coins: HashSet<&str> = HashSet::new();
        for pair in &config.pairs {
            let (base, quote) = pair.split_once('_').ok_or(Error::msg(format!(
                "Invalid pair format: {}. Expected format: BASE_QUOTE",
                pair
            )))?;

            coins.insert(base);
            coins.insert(quote);
        }

        let active_coins = currencies_client
            .get_added_currencies(protos::GetAddedCurrenciesRequest {})
            .await?
            .into_inner()
            .currency;

        for coin in coins {
            if !active_coins.contains(&coin.to_string()) {
                let asset = config
                    .assets
                    .get(coin)
                    .ok_or(Error::msg(format!("Asset {} not found in config", coin)))?;

                match asset {
                    Asset::Utxo { .. } => {
                        let request = protos::AddCurrencyRequest {
                            currency: coin.into(),
                            conf: Some(protos::add_currency_request::Conf::Lnd(
                                protos::LndConfiguration {
                                    lnd_channel: config.lnd_service_url.clone(),
                                    tls_cert: Some(TlsCert::CertPath(config.lnd_cert_dir.clone())),
                                    macaroon: Some(Macaroon::MacaroonPath(
                                        config.lnd_macaroon_dir.clone(),
                                    )),
                                },
                            )),
                        };
                        currencies_client.add_currency(request).await?;
                    }
                    Asset::Evm { address, .. } => {
                        let request = protos::AddCurrencyRequest {
                            currency: coin.into(),
                            conf: Some(protos::add_currency_request::Conf::Connext(
                                protos::ConnextConfiguration {
                                    connext_channel: config.connext_service_url.clone(),
                                    event_resolver: config.connext_event_resolver_url.clone(),
                                    token_address: address.to_string(),
                                },
                            )),
                        };
                        currencies_client.add_currency(request).await?;
                    }
                }
            }
        }

        for pair in &config.pairs {
            let request = protos::EnableTradingPairRequest {
                pair_id: pair.to_string(),
            };
            trading_pairs_client.enable_trading_pair(request).await?;
        }

        debug!(
            "Hydranet | Active currencies: {:?}",
            currencies_client
                .get_added_currencies(protos::GetAddedCurrenciesRequest {})
                .await?
                .into_inner()
                .currency
        );

        debug!(
            "Hydranet | Active trading pairs: {:?}",
            trading_pairs_client
                .get_active_trading_pair(protos::GetActiveTradingPairRequest {})
                .await?
                .into_inner()
                .pair_id
        );

        let orderbook_update_sinks = Arc::new(Mutex::new(HashMap::new()));
        let trade_update_sinks = Arc::new(Mutex::new(HashMap::new()));
        let is_synced = Arc::new(AtomicBool::new(false));
        let sync_notifier = Arc::new(Notify::new());

        let orders_stream = orders_client
            .subscribe_orders(protos::SubscribeOrdersRequest {})
            .await?
            .into_inner();
        let own_orders_stream = orders_client
            .subscribe_own_orders(protos::SubscribeOrdersRequest {})
            .await?
            .into_inner();

        let orders_client = Arc::new(Mutex::new(orders_client));
        let order_id_map = Arc::new(Mutex::new(HashMap::new()));
        let stream_subscriptions_handle = tokio::spawn(streams_task(
            orders_client.clone(),
            orders_stream,
            own_orders_stream,
            orderbook_update_sinks.clone(),
            trade_update_sinks.clone(),
            is_synced.clone(),
            sync_notifier.clone(),
            order_id_map.clone()
        ));

        let client = Hydranet {
            orders_client,
            lnd_client: Mutex::new(lnd_client),
            connext_client,
            config,
            stream_subscriptions_handle,
            orderbook_update_sinks,
            trade_update_sinks,
            is_synced,
            sync_notifier,
            order_id_map
        };

        Ok(client)
    }
}

impl Drop for Hydranet {
    fn drop(&mut self) {
        self.stream_subscriptions_handle.abort();
    }
}

#[allow(clippy::too_many_arguments)]
async fn streams_task(
    orders_client: Arc<Mutex<OrdersClient<tonic::transport::Channel>>>,
    mut orders_stream: tonic::Streaming<protos::OrderUpdate>,
    mut own_orders_stream: tonic::Streaming<protos::OwnOrderUpdate>,
    orderbook_update_sinks: Arc<Mutex<OrderbookUpdateSinks>>,
    trade_update_sinks: Arc<Mutex<TradeUpdateSinks>>,
    is_synced: Arc<AtomicBool>,
    sync_notifier: Arc<Notify>,
    order_id_map: Arc<Mutex<HashMap<PairOrderId,String>>>
) -> (){
    let mut first_loop = true;
    loop {
        // reconnect if connection is closed
        if !first_loop {
            is_synced.store(false, std::sync::atomic::Ordering::Release);

            // wait 5 seconds before reconnecting
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            debug!("Reconnecting to Hydranet streams...");

            let mut orders_client = orders_client.lock().await;
            orders_stream = match orders_client
                .subscribe_orders(protos::SubscribeOrdersRequest {})
                .await
            {
                Ok(stream) => stream.into_inner(),
                Err(e) => {
                    error!("Error subscribing to orders stream: {}", e);
                    continue;
                }
            };

            own_orders_stream = match orders_client
                .subscribe_own_orders(protos::SubscribeOrdersRequest {})
                .await
            {
                Ok(stream) => stream.into_inner(),
                Err(e) => {
                    error!("Error subscribing to own orders stream: {}", e);
                    continue;
                }
            };

            debug!("Reconnected to Hydranet streams");
        }

        is_synced.store(true, std::sync::atomic::Ordering::Release);
        sync_notifier.notify_waiters();

        // // TODO: remove partial trades map when lssd is fixed (needed to save id of partial trades to match the bugged completed order)
        // let mut active_orders: HashMap<String, protos::Order> = HashMap::new(); // order_id -> order
        // let mut partial_trades_map: HashMap<String, (String, Vec<String>)> = HashMap::new(); // order_id -> (pair, partial_trades_ids)

        loop {
            first_loop = false;

            tokio::select! {
                order_msg = orders_stream.next() => {
                    match order_msg {
                        Some(Ok(msg)) => {
                            if let Err(e) = handle_order_update_message(
                                orderbook_update_sinks.clone(),
                                msg,
                            )
                            .await
                            {
                                error!("{}", e);
                            }
                        }
                        Some(Err(e)) => {
                            error!("Error while receiving order update: {}", e);
                            break;
                        }
                        None => break,
                    }
                }
                own_order_msg = own_orders_stream.next() => {
                    match own_order_msg {
                        Some(Ok(msg)) => {
                            if let Err(e) = handle_own_order_update_message(
                                trade_update_sinks.clone(),
                                order_id_map.clone(),
                                msg,
                            )
                            .await
                            {
                                error!("{}", e);
                            }
                        }
                        Some(Err(e)) => {
                            error!("Error while receiving own order update: {}", e);
                            break;
                        }
                        None => break,
                    }
                }
            }
        }

        error!("Hydranet streams connection closed");



        // close and drain all sinks
        for (_, sinks) in orderbook_update_sinks.lock().await.drain() {
            for sink in sinks {
                sink.close_channel();
            }
        }

        for (_, sinks) in trade_update_sinks.lock().await.drain() {
            for sink in sinks {
                sink.close_channel();
            }
        }
    }
}

async fn handle_order_update_message(
    orderbook_update_sinks: Arc<Mutex<OrderbookUpdateSinks>>,
    message: protos::OrderUpdate,
) -> Result<(), Error> {
    let (pair, orderbook_update) = match message.update {
        Some(protos::order_update::Update::OrderAdded(order_summary)) => {
            let size: Decimal = order_summary
                .funds
                .ok_or(Error::msg("Funds not found"))?
                .try_into()?;
            let price: Decimal = order_summary
                .price
                .ok_or(Error::msg("Price not found"))?
                .try_into()?;
            let side = if order_summary.side == protos::OrderSide::Sell as i32 {
                Side::Sell
            } else {
                Side::Buy
            };
            let order_update = OrderUpdate {
                price,
                size,
                side,
                update_type: OrderUpdateType::Added,
            };

            (
                order_summary.pair_id,
                OrderbookUpdate::OrdersUpdate {
                    timestamp_millis: order_summary.created_at * 1000,
                    orders_update: vec![order_update],
                },
            )
        }
        Some(protos::order_update::Update::OrderRemoval(order_summary)) => {
            let size: Decimal = order_summary
                .funds
                .ok_or(Error::msg("Funds not found"))?
                .try_into()?;
            let price: Decimal = order_summary
                .price
                .ok_or(Error::msg("Price not found"))?
                .try_into()?;
            let side = if order_summary.side == protos::OrderSide::Sell as i32 {
                Side::Sell
            } else {
                Side::Buy
            };

            let order_update = OrderUpdate {
                price,
                size,
                side,
                update_type: OrderUpdateType::Removed,
            };

            (
                order_summary.pair_id,
                OrderbookUpdate::OrdersUpdate {
                    timestamp_millis: order_summary.created_at * 1000,
                    orders_update: vec![order_update],
                },
            )
        }
        None => return Err(Error::msg("Empty order update")),
    };

    let mut sinks = orderbook_update_sinks.lock().await;
    if let Some(sinks) = sinks.get_mut(&pair) {
        sinks.retain(|sink| {
            if sink.unbounded_send(orderbook_update.clone()).is_err() {
                sink.close_channel();
                false
            } else {
                true
            }
        });
    }

    Ok(())
}

async fn handle_own_order_update_message(
    trade_update_sinks: Arc<Mutex<TradeUpdateSinks>>,
    order_id_map: Arc<Mutex<HashMap<PairOrderId,String>>>,
    message: protos::OwnOrderUpdate,
) -> Result<(), Error> {
    let mut trade_updates = vec![];
    let pair = match message.update {
        Some(protos::own_order_update::Update::OrderAdded(order)) => {
            let pair = order.pair_id.clone();
            let old_order_id = order.old_order_id.clone();
            let new_order_id = order.order_id.clone();
            let pair_order_id = PairOrderId {
                pair: pair.clone(),
                order_id: new_order_id.clone()
            };
            let mut order_id_map = order_id_map.lock().await;

            if old_order_id.is_empty() {
               
                order_id_map.insert(pair_order_id, new_order_id.clone());

                trade_updates.push(TradeOrderUpdate::Added(proto_order_to_trade_order(&order)?));
            } else {
                order_id_map.clone().iter().for_each(|(k,v)| {
                    if v == &old_order_id {
                        order_id_map.insert(k.clone(), new_order_id.clone());
                    }
                });
               



            }
            

            order.pair_id
        }
        Some(protos::own_order_update::Update::OrderChanged(order)) => {
      
            let original_amount = Decimal::from_str(&order.funds.clone().unwrap().value).unwrap_or_default();

            let mut sum_closed_portions = Decimal::zero();
            for closed_portion in &order.closed {

                sum_closed_portions += Decimal::from_str(&closed_portion.amount.clone().unwrap().value).unwrap_or_default();

            }

            if original_amount != sum_closed_portions {
                let order_id_map = order_id_map.lock().await;
                let original_order_id = order_id_map.iter().find_map(|(k,v)| {
                    if v == &order.old_order_id {
                        Some(k.clone())
                    } else {
                        None
                    }
                }).unwrap_or(PairOrderId{
                    pair: order.pair_id.clone(),
                    order_id: order.old_order_id.clone()
                
                });

                let mut trade_order = proto_order_to_trade_order(&order)?;
                trade_order.id = original_order_id.order_id.clone();             
                trade_updates.push(TradeOrderUpdate::Changed(trade_order));
            }
            

            order.pair_id
        }
        Some(protos::own_order_update::Update::OrderCompleted(order_id)) => {

            let mut order_id_map = order_id_map.lock().await;
            if let Some(original_order_id) = order_id_map.iter().find_map(|(k,v)| {
                if v == &order_id {
                    Some(k.clone())
                } else {
                    None
                }
            }) {
                trade_updates.push(TradeOrderUpdate::Completed {
                    order_id: original_order_id.order_id.clone(),
                    timestamp_millis: SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)?
                        .as_millis() as u64,
                });
                order_id_map.remove(&original_order_id);
                original_order_id.pair

            } else {
                return Err(Error::msg(format!(
                    "Completed order {} not found in order_id_map",
                    order_id
                )));
            }
        }
        Some(protos::own_order_update::Update::OrderCanceled(order_id)) => {
            let mut order_id_map = order_id_map.lock().await;
            if let Some(original_order_id) = order_id_map.iter().find_map(|(k,v)| {
                if v == &order_id {
                    Some(k.clone())
                } else {
                    None
                }
            }) {
                trade_updates.push(TradeOrderUpdate::Canceled  {
                    order_id: original_order_id.order_id.clone(),
                    timestamp_millis: SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)?
                        .as_millis() as u64,
                });
                order_id_map.remove(&original_order_id);
                original_order_id.pair

            } else {
                return Err(Error::msg(format!(
                    "Cancelled order {} not found in order_id_map",
                    order_id
                )));
            }
        }
        None => return Err(Error::msg("Empty own order update")),
    
        };

    let mut sinks = trade_update_sinks.lock().await;
    if let Some(sinks) = sinks.get_mut(&pair) {
        sinks.retain(|sink| {
            for trade_update in &trade_updates {
                if sink.unbounded_send(trade_update.clone()).is_err() {
                    sink.close_channel();
                    return false;
                }
            }

            true
        });
    }

    Ok(())
}
