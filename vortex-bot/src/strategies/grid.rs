use std::{hash::Hash, sync::Arc};

use anyhow::Error;
use futures::StreamExt;
use rust_decimal::{prelude::FromPrimitive, Decimal};
use serde::Deserialize;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use vortex_core::{
    api::CreateOrderError,
    market::Market,
    types::{MarketUpdate, Side, TradeOrder, TradeOrderUpdate},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct GridConfig {
    pub grid_profit: Decimal,
    pub grid_amount: Decimal,
    pub min_price: Decimal,
    pub max_price: Decimal,
    pub get_mid_price_from_min_max_avg: bool
}

pub async fn start_grid_strategy(
    market: Arc<Market>,
    config: GridConfig,
) -> Result<JoinHandle<()>, Error> {
    let mut grid_orders = Vec::<(Decimal, Option<TradeOrder>)>::new(); // (price_level, order)

    // set price levels
    let mut price_level = config.min_price;
    while price_level <= config.max_price {
        price_level = price_level.round_dp(market.info().price_precision as u32);
        grid_orders.push((price_level, None));
        price_level *= Decimal::ONE + config.grid_profit;
    }

    let grid_strategy_task = async move {
        'grid_loop: loop {
            market.wait_sync().await;

            let mut market_update_stream = market.update_stream();

            let starting_price = if let Some(starting_price) = market.orderbook().get_mid_price() {
                starting_price
            } else if let Some(best_ask) = market.orderbook().get_best_ask() {
                best_ask.price
            } else if let Some(best_bid) = market.orderbook().get_best_bid() {
                best_bid.price
            } else  if config.get_mid_price_from_min_max_avg  {
                (config.min_price + config.max_price).checked_div(Decimal::from_i32(2).unwrap()).unwrap()
            }
            else {
                warn!(
                    "Failed to get starting price for {} market on {} exchange",
                    market.pair(),
                    market.exchange()
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                continue;
            };
            debug!("starting_price: {}", starting_price);

            // set cursor to the price level closest to the starting price and count buy/sell holes (after previous iteration)
            let mut starting_position: usize = 0;
            let mut closest_price_level_distance = Decimal::MAX;
            let mut buy_holes = 0;
            let mut sell_holes = 0;
            for (position, (price_level, order)) in grid_orders.iter_mut().enumerate() {
                let distance = (starting_price - *price_level).abs();
                if distance < closest_price_level_distance {
                    closest_price_level_distance = distance;
                    starting_position = position;

                    // distance is shortening (count buy holes)
                    if let Some(buy_order) = order {
                        if market
                            .pending_orders()
                            .get_order_by_id(&buy_order.id)
                            .is_none()
                        {
                            *order = None;
                            buy_holes += 1;
                        }
                    } else {
                        buy_holes += 1;
                    }
                } else if let Some(sell_order) = order {
                    // distance is increasing (count sell holes)

                    if market
                        .pending_orders()
                        .get_order_by_id(&sell_order.id)
                        .is_none()
                    {
                        *order = None;
                        sell_holes += 1;
                    }
                } else {
                    sell_holes += 1;
                }
            }

            // cancel buy_holes number of buy orders starting from the farthest price level (lowest price)
            for (_, order) in grid_orders.iter_mut() {
                if buy_holes == 0 {
                    break;
                }

                if let Some(buy_order) = order {
                    if let Err(e) = market.cancel_order(&buy_order.id).await {
                        warn!(
                            "Failed to cancel buy order for {} market on {} exchange: {:?}",
                            market.pair(),
                            market.exchange(),
                            e
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        continue 'grid_loop;
                    }

                    *order = None;
                    buy_holes -= 1;
                }
            }

            // cancel sell_holes number of sell orders starting from the farthest price level (highest price)
            for (_, order) in grid_orders.iter_mut().rev() {
                if sell_holes == 0 {
                    break;
                }

                if let Some(sell_order) = order {
                    if let Err(e) = market.cancel_order(&sell_order.id).await {
                        warn!(
                            "Failed to cancel sell order for {} market on {} exchange: {:?}",
                            market.pair(),
                            market.exchange(),
                            e
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        continue 'grid_loop;
                    }

                    *order = None;
                    sell_holes -= 1;
                }
            }

            // fill all the buy holes starting from the closest price level until we have insufficient funds
            for (price_level, order) in grid_orders[..starting_position].iter_mut().rev() {
                if order.is_none() {
                    let new_buy_order = match market
                        .create_limit_order(Side::Buy, config.grid_amount, *price_level)
                        .await
                    {
                        Ok(order) => order,
                        Err(e) => match e {
                            CreateOrderError::InsufficientFunds => {
                                warn!(
                                    "Insufficient funds for {} market on {} exchange: {:?}",
                                    market.pair(),
                                    market.exchange(),
                                    e
                                );
                                break;
                            }
                            CreateOrderError::Other(e) => {
                                warn!(
                                    "Failed to create buy order for {} market on {} exchange: {:?}",
                                    market.pair(),
                                    market.exchange(),
                                    e
                                );
                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                                continue 'grid_loop;
                            }
                        },
                    };

                    *order = Some(new_buy_order);
                }
            }

            // fill all the sell holes starting from the closest price level until we have insufficient funds
            for (price_level, order) in grid_orders[(starting_position + 1)..].iter_mut() {
                if order.is_none() {
                    let new_sell_order = match market
                        .create_limit_order(Side::Sell, config.grid_amount, *price_level)
                        .await
                    {
                        Ok(order) => order,
                        Err(e) => match e {
                            CreateOrderError::InsufficientFunds => {
                                warn!(
                                    "Insufficient funds for {} market on {} exchange: {:?}",
                                    market.pair(),
                                    market.exchange(),
                                    e
                                );
                                break;
                            }
                            CreateOrderError::Other(e) => {
                                warn!(
                                    "Failed to create sell order for {} market on {} exchange: {:?}",
                                    market.pair(),
                                    market.exchange(),
                                    e
                                );
                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                                continue 'grid_loop;
                            }
                        },
                    };

                    *order = Some(new_sell_order);
                }
            }

            info!(
                "Grid between {} and {} for {} market on {} exchange created",
                config.min_price,
                config.max_price,
                market.pair(),
                market.exchange()
            );

            while let Some(market_update) = market_update_stream.next().await {
                let trade_order_update =
                    if let MarketUpdate::TradeOrder(trade_order_update) = market_update {
                        trade_order_update
                    } else {
                        continue;
                    };

                match trade_order_update {
                    TradeOrderUpdate::Completed { order_id, .. } => {
                        let completed_grid_order = grid_orders.iter_mut().enumerate().find_map(
                            |(position, (_, order))| {
                                if let Some(trade_order) = order.clone() {
                                    if trade_order.id == order_id {
                                        // remove order from grid
                                        *order = None;

                                        Some((position, trade_order))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            },
                        );

                        if let Some((price_level_position, order)) = completed_grid_order {
                            match order.side {
                                Side::Buy => {
                                    let sell_price = match grid_orders.get(price_level_position + 1)
                                    {
                                        Some((price_level, _)) => price_level,
                                        None => {
                                            info!(
                                                "Grid buy order for {} market on {} exchange completed at {}: no sell order to place",
                                                market.pair(),
                                                market.exchange(),
                                                order.price
                                            );
                                            continue;
                                        }
                                    };

                                    info!(
                                        "Grid buy order for {} market on {} exchange completed at {}: placing sell order at {}",
                                        market.pair(),
                                        market.exchange(),
                                        order.price,
                                        sell_price
                                    );

                                    let new_sell_order = match market
                                        .create_limit_order(
                                            Side::Sell,
                                            config.grid_amount,
                                            *sell_price,
                                        )
                                        .await
                                    {
                                        Ok(order) => order,
                                        Err(e) => {
                                            warn!(
                                                "Failed to create sell order for {} market on {} exchange: {:?}",
                                                market.pair(),
                                                market.exchange(),
                                                e
                                            );
                                            tokio::time::sleep(tokio::time::Duration::from_secs(1))
                                                .await;
                                            continue 'grid_loop;
                                        }
                                    };

                                    grid_orders[price_level_position + 1].1 = Some(new_sell_order);
                                }
                                Side::Sell => {
                                    let buy_price = match grid_orders.get(price_level_position - 1)
                                    {
                                        Some((price_level, _)) => price_level,
                                        None => {
                                            info!(
                                                "Grid sell order for {} market on {} exchange completed at {}: no buy order to place",
                                                market.pair(),
                                                market.exchange(),
                                                order.price
                                            );
                                            continue;
                                        }
                                    };

                                    info!(
                                        "Grid sell order for {} market on {} exchange completed at {}: placing buy order at {}",
                                        market.pair(),
                                        market.exchange(),
                                        order.price,
                                        buy_price
                                    );

                                    let new_buy_order = match market
                                        .create_limit_order(
                                            Side::Buy,
                                            config.grid_amount,
                                            *buy_price,
                                        )
                                        .await
                                    {
                                        Ok(order) => order,
                                        Err(e) => {
                                            warn!(
                                                "Failed to create buy order for {} market on {} exchange: {:?}",
                                                market.pair(),
                                                market.exchange(),
                                                e
                                            );
                                            tokio::time::sleep(tokio::time::Duration::from_secs(1))
                                                .await;
                                            continue 'grid_loop;
                                        }
                                    };

                                    grid_orders[price_level_position - 1].1 = Some(new_buy_order);
                                }
                            }
                        } else {
                            // completed order in not a grid order, thus we might have balance to fill any of the holes
                            let mut first_buy_hole_position = None;
                            let mut first_sell_hole_position = None;
                            for (index, window) in grid_orders.windows(3).enumerate() {
                                if let [(_, None), (_, None), (_, Some(order))] = window {
                                    match order.side {
                                        Side::Buy => {
                                            first_buy_hole_position = Some(index + 1);
                                        }
                                        Side::Sell => {
                                            first_buy_hole_position = Some(index);
                                        }
                                    }
                                } else if let [(_, Some(order)), (_, None), (_, None)] = window {
                                    match order.side {
                                        Side::Buy => {
                                            first_sell_hole_position = Some(index + 2);
                                        }
                                        Side::Sell => {
                                            first_sell_hole_position = Some(index + 1);
                                        }
                                    }
                                }
                            }

                            if let Some(first_buy_hole_position) = first_buy_hole_position {
                                for (price_level, order) in
                                    grid_orders[..first_buy_hole_position].iter_mut().rev()
                                {
                                    if order.is_none() {
                                        let new_buy_order = match market
                                            .create_limit_order(
                                                Side::Buy,
                                                config.grid_amount,
                                                *price_level,
                                            )
                                            .await
                                        {
                                            Ok(order) => order,
                                            Err(_) => continue,
                                        };

                                        *order = Some(new_buy_order);

                                        info!("Placed missing buy order at {}", price_level);
                                    }
                                }
                            } else if let Some(first_sell_hole_position) = first_sell_hole_position
                            {
                                for (price_level, order) in
                                    grid_orders[first_sell_hole_position..].iter_mut()
                                {
                                    if order.is_none() {
                                        let new_sell_order = match market
                                            .create_limit_order(
                                                Side::Sell,
                                                config.grid_amount,
                                                *price_level * (Decimal::ONE + config.grid_profit),
                                            )
                                            .await
                                        {
                                            Ok(order) => order,
                                            Err(_) => continue,
                                        };

                                        *order = Some(new_sell_order);

                                        info!("Placed missing sell order at {}", price_level);
                                    }
                                }
                            }
                        }
                    }
                    TradeOrderUpdate::Canceled { order_id, .. } => {
                        // remove order from grid
                        grid_orders.iter_mut().find_map(|(_, order)| {
                            if let Some(trade_order) = order {
                                if trade_order.id == order_id {
                                    *order = None;

                                    Some(())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        });
                    }
                    TradeOrderUpdate::Changed(new_order) => {
                        // update order in grid
                        grid_orders.iter_mut().find_map(|(_, order)| {
                            if let Some(trade_order) = order.clone() {
                                if trade_order.id == new_order.id {
                                    *order = Some(new_order.clone());

                                    Some(())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        });
                    }
                    _ => {}
                }
            }

            warn!(
                "Update stream for {} market on {} exchange ended",
                market.pair(),
                market.exchange()
            );
        }
    };

    Ok(tokio::spawn(grid_strategy_task))
}
