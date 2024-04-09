use anyhow::Error;
use async_trait::async_trait;
use futures::StreamExt;
use rust_decimal::Decimal;
use vortex_core::api::OrderFetcher;
use vortex_core::types::{Side, TradeOrder, TradeOrderUpdateStream, TradeOrders, Type};

use crate::protos;
use crate::Hydranet;

#[async_trait]
impl OrderFetcher for Hydranet {
    async fn get_orders_pending(&self, pair: &str) -> Result<TradeOrders, Error> {
        let request = protos::ListOwnOrdersRequest {
            pair_id: pair.to_string(),
        };

        let mut orders_client = self.orders_client.lock().await;
        let orders = orders_client
            .list_own_orders(request.clone())
            .await?
            .into_inner()
            .orders;

        let mut buys = Vec::<TradeOrder>::new();
        let mut sells = Vec::<TradeOrder>::new();

        for order in orders {
            let amount: Decimal = order
                .funds
                .ok_or(Error::msg("Funds not found"))?
                .try_into()?;
            let price: Decimal = order
                .price
                .ok_or(Error::msg("Price not found"))?
                .try_into()?;
            let mut executed = Decimal::ZERO;
            for trade in order.closed {
                executed +=
                    Decimal::try_from(trade.amount.ok_or(Error::msg("Amount not found"))?.clone())?;
            }
            let left = amount - executed;
            let side = if order.side == protos::OrderSide::Sell as i32 {
                Side::Sell
            } else {
                Side::Buy
            };

            let trade_order = TradeOrder {
                id: order.order_id,
                timestamp_millis: order.created_at * 1000,
                side,
                r#type: Type::Limit,
                price,
                amount,
                left,
                executed,
            };

            match side {
                Side::Sell => sells.push(trade_order),
                Side::Buy => buys.push(trade_order),
            }
        }

        // sort orders by price
        sells.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap());
        buys.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap());

        Ok(TradeOrders { buys, sells })
    }

    async fn subscribe_orders_pending(&self, pair: &str) -> Result<TradeOrderUpdateStream, Error> {
        let (trade_update_sink, trade_update_stream) = futures::channel::mpsc::unbounded();
        self.trade_update_sinks
            .lock()
            .await
            .entry(pair.to_string())
            .or_default()
            .push(trade_update_sink);

        Ok(trade_update_stream.boxed())
    }
}
