use std::{collections::HashSet, sync::Arc};

use anyhow::Error;
use rand::{rngs::OsRng, Rng};
use rust_decimal::{
    prelude::{FromPrimitive, ToPrimitive},
    Decimal,
};
use serde::Deserialize;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vortex_core::{api::CreateOrderError, market::Market, types::Side};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct VolumeMakerConfig {
    pub order_amount: Decimal,
    pub order_amount_randomness: Decimal,
    pub order_frequency_secs: Decimal,
    pub order_frequency_randomness: Decimal,
}

pub async fn start_volume_maker_strategy(
    market: Arc<Market>,
    config: VolumeMakerConfig,
) -> Result<JoinHandle<()>, Error> {
    let volume_maker_strategy_task = async move {
        let mut open_order_ids = HashSet::<String>::new();

        loop {
            market.wait_sync().await;

            let sleep_secs = config.order_frequency_secs
                * (Decimal::ONE
                    + config.order_frequency_randomness
                        * Decimal::from_f64(OsRng.gen_range(-1.0..=1.0)).unwrap());

            tokio::time::sleep(tokio::time::Duration::from_secs(
                sleep_secs.to_u64().unwrap(),
            ))
            .await;

            // cancel open orders from previous iterations
            for order_id in open_order_ids.clone() {
                if market.pending_orders().get_order_by_id(&order_id).is_some() {
                    match market.cancel_order(&order_id).await {
                        Ok(_) => {
                            info!(
                                "Canceled order {} for {} market on {} exchange",
                                order_id,
                                market.pair(),
                                market.exchange()
                            );
                            open_order_ids.remove(&order_id);
                        }
                        Err(e) => {
                            warn!(
                                "Failed to cancel order {} for {} market on {} exchange: {:?}",
                                order_id,
                                market.pair(),
                                market.exchange(),
                                e
                            );
                        }
                    }
                } else {
                    open_order_ids.remove(&order_id);
                }
            }

            let order_amount = config.order_amount
                * (Decimal::ONE
                    + config.order_amount_randomness
                        * Decimal::from_f64(OsRng.gen_range(-1.0..=1.0)).unwrap());

            let mid_price = match market.orderbook().get_mid_price() {
                Some(mid_price) => mid_price,
                None => {
                    warn!(
                        "Failed to get mid price for {} market on {} exchange",
                        market.pair(),
                        market.exchange()
                    );
                    continue;
                }
            };

            let sell_balance = market.base_balance().free;
            let buy_balance = market.quote_balance().free / mid_price;

            // place order only if we have enough balance for both trades
            if sell_balance < order_amount && buy_balance < order_amount {
                warn!(
                    "Insufficient funds for {} market on {} exchange",
                    market.pair(),
                    market.exchange()
                );
                continue;
            }

            let do_buy = OsRng.gen_bool(0.5);

            if do_buy {
                match market
                    .create_limit_order(Side::Buy, order_amount, mid_price)
                    .await
                {
                    Ok(order) => {
                        info!(
                            "Created buy order of {} {} at {} for {} market on {} exchange",
                            order.amount,
                            market.base(),
                            order.price,
                            market.pair(),
                            market.exchange()
                        );
                        open_order_ids.insert(order.id);
                    }
                    Err(CreateOrderError::InsufficientFunds) => {
                        warn!(
                            "Insufficient funds for {} market on {} exchange",
                            market.pair(),
                            market.exchange()
                        );
                        continue;
                    }
                    Err(e) => {
                        warn!(
                            "Failed to create buy order for {} market on {} exchange: {:?}",
                            market.pair(),
                            market.exchange(),
                            e
                        );
                        continue;
                    }
                }
            } else {
                match market
                    .create_limit_order(Side::Sell, order_amount, mid_price)
                    .await
                {
                    Ok(order) => {
                        info!(
                            "Created sell order of {} {} at {} for {} market on {} exchange",
                            order.amount,
                            market.base(),
                            order.price,
                            market.pair(),
                            market.exchange()
                        );
                        open_order_ids.insert(order.id);
                    }
                    Err(CreateOrderError::InsufficientFunds) => {
                        warn!(
                            "Insufficient funds for {} market on {} exchange",
                            market.pair(),
                            market.exchange()
                        );
                        continue;
                    }
                    Err(e) => {
                        warn!(
                            "Failed to create sell order for {} market on {} exchange: {:?}",
                            market.pair(),
                            market.exchange(),
                            e
                        );
                        continue;
                    }
                }
            }

            let sleep_secs = config.order_frequency_secs
                * (Decimal::ONE
                    + config.order_frequency_randomness
                        * Decimal::from_f64(OsRng.gen_range(-1.0..=1.0)).unwrap());

            tokio::time::sleep(tokio::time::Duration::from_secs(
                sleep_secs.to_u64().unwrap(),
            ))
            .await;

            let order_amount = config.order_amount
                * (Decimal::ONE
                    + config.order_amount_randomness
                        * Decimal::from_f64(OsRng.gen_range(-1.0..=1.0)).unwrap());

            // create a limit order at the opposite side and at the same price as the previous order
            if do_buy {
                match market
                    .create_limit_order(Side::Sell, order_amount, mid_price)
                    .await
                {
                    Ok(order) => {
                        info!(
                            "Created sell order of {} {} at {} for {} market on {} exchange",
                            order.amount,
                            market.base(),
                            order.price,
                            market.pair(),
                            market.exchange()
                        );
                        open_order_ids.insert(order.id);
                    }
                    Err(CreateOrderError::InsufficientFunds) => {
                        warn!(
                            "Insufficient funds for {} market on {} exchange",
                            market.pair(),
                            market.exchange()
                        );
                        continue;
                    }
                    Err(e) => {
                        warn!(
                            "Failed to create sell order for {} market on {} exchange: {:?}",
                            market.pair(),
                            market.exchange(),
                            e
                        );
                        continue;
                    }
                }
            } else {
                match market
                    .create_limit_order(Side::Buy, order_amount, mid_price)
                    .await
                {
                    Ok(order) => {
                        info!(
                            "Created buy order of {} {} at {} for {} market on {} exchange",
                            order.amount,
                            market.base(),
                            order.price,
                            market.pair(),
                            market.exchange()
                        );
                        open_order_ids.insert(order.id);
                    }
                    Err(CreateOrderError::InsufficientFunds) => {
                        warn!(
                            "Insufficient funds for {} market on {} exchange",
                            market.pair(),
                            market.exchange()
                        );
                        continue;
                    }
                    Err(e) => {
                        warn!(
                            "Failed to create buy order for {} market on {} exchange: {:?}",
                            market.pair(),
                            market.exchange(),
                            e
                        );
                        continue;
                    }
                }
            }
        }
    };

    Ok(tokio::spawn(volume_maker_strategy_task))
}
