use std::{
    sync::{atomic::AtomicU64, Arc},
    time::SystemTime,
};

use anyhow::Error;
use futures::StreamExt;
use rust_decimal::{prelude::FromPrimitive, Decimal};
use serde::Deserialize;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vortex_core::{
    market::Market,
    types::{Info, MarketUpdate, Orderbook, Side},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct ArbitrageConfig {
    pub min_profit: Decimal,
    pub max_order_amount: Decimal,
}

// TODO: check arb also with remote balances (for hydranet exchange)

pub async fn start_arbitrage_strategy(
    markets: Vec<Arc<Market>>,
    config: ArbitrageConfig,
) -> Result<Vec<JoinHandle<()>>, Error> {
    if markets.len() < 2 {
        return Err(Error::msg("Arbitrage strategy requires at least 2 markets"));
    }

    let mut tasks = Vec::<JoinHandle<()>>::new();

    // timestamp of the last arbitrage trade that was executed in any of the markets
    let last_arb_trade_timestamp_millis = Arc::new(AtomicU64::new(0));

    // spawn a task for every combination of markets
    for (i, market_a) in markets.iter().enumerate() {
        for (j, market_b) in markets.iter().enumerate() {
            if i == j {
                continue;
            }

            let market_a = market_a.clone();
            let market_b = market_b.clone();
            let last_arb_trade_timestamp_millis = last_arb_trade_timestamp_millis.clone();

            let arbitrage_strategy_task = async move {
                'arb_loop: loop {
                    market_a.wait_sync().await;
                    market_b.wait_sync().await;

                    let mut market_a_update_stream = market_a.update_stream();
                    let mut market_b_update_stream = market_b.update_stream();

                    // timestamp of the last successful arbitrage trade (both orders placed) that was executed in this pair of markets
                    let last_successful_arb_trade_timestamp_millis = Arc::new(AtomicU64::new(0));

                    loop {
                        let update_timestamp_ms = tokio::select! {
                            market_a_update = market_a_update_stream.next() => {
                                if let Some(update) = market_a_update {
                                    match update {
                                        MarketUpdate::Orderbook(_) => update.timestamp_millis(),
                                        _ => continue,
                                    }
                                } else {
                                    warn!("Update stream for {} market on {} exchange ended", market_a.pair(), market_a.exchange());
                                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                                    continue 'arb_loop;
                                }
                            },
                            market_b_update = market_b_update_stream.next() => {
                                if let Some(update) = market_b_update {
                                    match update {
                                        MarketUpdate::Orderbook(_) => update.timestamp_millis(),
                                        _ => continue,
                                    }
                                } else {
                                    warn!("Update stream for {} market on {} exchange ended", market_b.pair(), market_b.exchange());
                                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                                    continue 'arb_loop;
                                }
                            },
                        };

                        let pair_balance_a = PairBalance {
                            base: market_a.base_balance().free,
                            quote: market_a.quote_balance().free,
                        };

                        let market_state_a = MarketState {
                            info: market_a.info().clone(),
                            orderbook: market_a.orderbook(),
                            pair_balance: pair_balance_a,
                        };

                        let pair_balance_b = PairBalance {
                            base: market_b.base_balance().free,
                            quote: market_b.quote_balance().free,
                        };

                        let market_state_b = MarketState {
                            info: market_b.info().clone(),
                            orderbook: market_b.orderbook(),
                            pair_balance: pair_balance_b,
                        };

                        // TODO: price feed for USD price
                        let usd_base_price =
                            market_a.orderbook().get_mid_price().unwrap_or(Decimal::ONE);

                        let arb_opportunity = check_a_to_b_arbitrage(
                            market_state_a,
                            market_state_b,
                            config.min_profit,
                            config.max_order_amount,
                            usd_base_price,
                        );

                        if let Some(ArbOpportunity {
                            buy_amount,
                            sell_amount,
                            buy_price,
                            sell_price,
                        }) = arb_opportunity
                        {
                            // check if a trade involving these markets has already been executed
                            // after the update timestamp
                            let last_trade_timestamp_millis = market_a
                                .last_trade_timestamp_millis()
                                .await
                                .max(market_b.last_trade_timestamp_millis().await);

                            if update_timestamp_ms <= last_trade_timestamp_millis {
                                continue;
                            }

                            // check if an arbitrage trade has already been executed in any of the markets after the update timestamp
                            if update_timestamp_ms
                                <= last_arb_trade_timestamp_millis
                                    .load(std::sync::atomic::Ordering::Acquire)
                            {
                                continue;
                            }

                            // check if we already successfully placed orders for this arbitrage opportunity
                            // NOTE: with both orders placed, both orderbooks should be updated
                            let last_successful_trade_timestamp_millis =
                                last_successful_arb_trade_timestamp_millis
                                    .load(std::sync::atomic::Ordering::Acquire);

                            if market_a.orderbook().timestamp_millis
                                <= last_successful_trade_timestamp_millis
                                || market_b.orderbook().timestamp_millis
                                    <= last_successful_trade_timestamp_millis
                            {
                                continue;
                            }

                            // set the last arbitrage trade timestamp to the current timestamp to prevent
                            // multiple arbitrage trades from being executed in parallel
                            last_arb_trade_timestamp_millis.store(
                                SystemTime::now()
                                    .duration_since(SystemTime::UNIX_EPOCH)
                                    .unwrap()
                                    .as_millis() as u64,
                                std::sync::atomic::Ordering::Release,
                            );

                            info!("Arbitrage opportunity between {} market on {} exchange and {} market on {} exchange: buy {} {} at {} on {} exchange, sell {} {} at {} on {} exchange", market_a.pair(), market_a.exchange(), market_b.pair(), market_b.exchange(), buy_amount, market_a.info().base, buy_price, market_a.exchange(), sell_amount, market_b.info().base, sell_price, market_b.exchange());

                            // place order where we have less balance first, and then the other order only if the first one succeeds
                            if market_b.base_balance().free
                                < market_a.quote_balance().free / buy_price
                            {
                                // place sell order first
                                match market_b
                                    .create_limit_order(Side::Sell, sell_amount, sell_price)
                                    .await
                                {
                                    Ok(order) => {
                                        last_arb_trade_timestamp_millis.fetch_max(
                                            order.timestamp_millis,
                                            std::sync::atomic::Ordering::Release,
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Failed to create sell order on {} exchange: {}",
                                            market_b.exchange(),
                                            e
                                        );
                                        continue;
                                    }
                                }

                                // place buy order second
                                match market_a
                                    .create_limit_order(Side::Buy, buy_amount, buy_price)
                                    .await
                                {
                                    Ok(order) => {
                                        last_arb_trade_timestamp_millis.fetch_max(
                                            order.timestamp_millis,
                                            std::sync::atomic::Ordering::Release,
                                        );
                                        // set the last successful arbitrage trade timestamp to the order timestamp
                                        last_successful_arb_trade_timestamp_millis.store(
                                            order.timestamp_millis,
                                            std::sync::atomic::Ordering::Release,
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Failed to create buy order on {} exchange: {}",
                                            market_a.exchange(),
                                            e
                                        );
                                    }
                                }
                            } else {
                                // place buy order first
                                match market_a
                                    .create_limit_order(Side::Buy, buy_amount, buy_price)
                                    .await
                                {
                                    Ok(order) => {
                                        last_arb_trade_timestamp_millis.fetch_max(
                                            order.timestamp_millis,
                                            std::sync::atomic::Ordering::Release,
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Failed to create buy order on {} exchange: {}",
                                            market_a.exchange(),
                                            e
                                        );
                                        continue;
                                    }
                                }

                                // place sell order second
                                match market_b
                                    .create_limit_order(Side::Sell, sell_amount, sell_price)
                                    .await
                                {
                                    Ok(order) => {
                                        last_arb_trade_timestamp_millis.fetch_max(
                                            order.timestamp_millis,
                                            std::sync::atomic::Ordering::Release,
                                        );
                                        // set the last successful arbitrage trade timestamp to the order timestamp
                                        last_successful_arb_trade_timestamp_millis.store(
                                            order.timestamp_millis,
                                            std::sync::atomic::Ordering::Release,
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Failed to create sell order on {} exchange: {}",
                                            market_b.exchange(),
                                            e
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            };

            tasks.push(tokio::spawn(arbitrage_strategy_task));
        }
    }

    Ok(tasks)
}

struct PairBalance {
    pub base: Decimal,
    pub quote: Decimal,
}

struct MarketState {
    pub info: Info,
    pub orderbook: Orderbook,
    pub pair_balance: PairBalance,
}

struct ArbOpportunity {
    buy_amount: Decimal,
    sell_amount: Decimal,
    buy_price: Decimal,
    sell_price: Decimal,
}

/// Checks if there is an arbitrage opportunity between two markets. Returns an ArbOpportunity
/// if there is an arbitrage opportunity, indicating the amounts and prices of the limit orders
/// that should be placed to execute the arbitrage (buy on market A, sell on market B).
/// Returns None if there is no arbitrage opportunity or we don't have enough balance to execute it.
fn check_a_to_b_arbitrage(
    market_state_a: MarketState,
    market_state_b: MarketState,
    min_profit: Decimal,
    max_order_amount: Decimal,
    usd_base_price: Decimal,
) -> Option<ArbOpportunity> {
    let orderbook_a = market_state_a.orderbook;
    let orderbook_b = market_state_b.orderbook;
    let pair_balance_a = market_state_a.pair_balance;
    let pair_balance_b = market_state_b.pair_balance;

    let min_amount = market_state_a
        .info
        .min_amount
        .max(market_state_b.info.min_amount);
    let min_usd_amount = market_state_a
        .info
        .min_usd_amount
        .max(market_state_b.info.min_usd_amount);

    let mut ask_idx = 0;
    let mut bid_idx = 0;
    let mut best_profit = Decimal::ZERO;
    let mut amount = Decimal::ZERO;
    let mut new_amount = Decimal::ZERO;

    // iterate over the orderbooks starting from the best ask and best bid
    while ask_idx < orderbook_a.asks.len() || bid_idx < orderbook_b.bids.len() {
        let ask = orderbook_a.asks.get(ask_idx);
        let bid = orderbook_b.bids.get(bid_idx);

        // increase the amount by the size of the next order in the orderbook
        // (either ask or bid, whichever is smaller)
        match (ask, bid) {
            (Some(ask), Some(bid)) => {
                if bid.size < ask.size {
                    bid_idx += 1;
                    new_amount += bid.size;
                } else {
                    ask_idx += 1;
                    new_amount += ask.size;
                }
            }
            (Some(ask), None) => {
                ask_idx += 1;
                new_amount += ask.size;
            }
            (None, Some(bid)) => {
                bid_idx += 1;
                new_amount += bid.size;
            }
            (None, None) => break,
        };

        let usd_amount = new_amount * usd_base_price;
        if new_amount < min_amount || usd_amount < min_usd_amount {
            continue;
        }

        if new_amount > max_order_amount {
            new_amount = max_order_amount;
        }

        let avg_ask_price = match orderbook_a.get_average_ask_price(new_amount) {
            Some(avg_ask_price) => avg_ask_price,
            None => break,
        };
        let avg_bid_price = match orderbook_b.get_average_bid_price(new_amount) {
            Some(avg_bid_price) => avg_bid_price,
            None => break,
        };

        let new_profit = (avg_bid_price - avg_ask_price) / avg_ask_price;
        if new_profit >= best_profit {
            let max_amount = pair_balance_b
                .base
                .min(pair_balance_a.quote / avg_ask_price)
                * Decimal::from_f64(0.99).unwrap(); // 1% buffer to account for rounding

            let not_enough_balance = new_amount > max_amount;
            if not_enough_balance {
                // cap the amount to the maximum possible amount
                if max_amount > min_amount && max_amount * usd_base_price > min_usd_amount {
                    best_profit = new_profit;
                    amount = max_amount;
                }

                break;
            } else {
                best_profit = new_profit;
                amount = new_amount;

                if amount >= max_order_amount {
                    break;
                }
            }
        } else {
            break;
        }
    }

    if best_profit > min_profit {
        // get the highest ask that we are going to buy at and the lowest bid that we are going to sell at
        // (we are going to place limit orders at these prices)
        let buy_price = orderbook_a.get_max_ask_price(amount)?;
        let sell_price = orderbook_b.get_min_bid_price(amount)?;

        Some(ArbOpportunity {
            buy_amount: amount,
            sell_amount: amount,
            buy_price,
            sell_price,
        })
    } else {
        None
    }
}
