use std::time::SystemTime;

use anyhow::Error;
use async_trait::async_trait;
use futures::StreamExt;
use rust_decimal::Decimal;
use vortex_core::api::OrderbookFetcher;
use vortex_core::types::{Order, Orderbook, OrderbookUpdateStream};

use crate::protos;
use crate::Hydranet;

#[async_trait]
impl OrderbookFetcher for Hydranet {
    async fn get_orderbook(&self, pair: &str) -> Result<Orderbook, Error> {
        let request = protos::ListOrdersRequest {
            pair_id: pair.to_string(),
            last_known_price: 1,
            ..Default::default()
        };
        let mut orders_client = self.orders_client.lock().await;
        let orders = orders_client
            .list_orders(request.clone())
            .await?
            .into_inner()
            .orders;

        let mut asks = vec![];
        let mut bids = vec![];
        for order in orders {
            let size: Decimal = order
                .funds
                .ok_or(Error::msg("Funds not found"))?
                .try_into()?;
            let price: Decimal = order
                .price
                .ok_or(Error::msg("Price not found"))?
                .try_into()?;

            if order.side == protos::OrderSide::Sell as i32 {
                asks.push(Order { price, size });
            } else {
                bids.push(Order { price, size });
            }
        }

        // sort orders by price
        asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap());
        bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap());

        Ok(Orderbook {
            timestamp_millis: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            asks,
            bids,
        })
    }

    async fn subscribe_orderbook(&self, pair: &str) -> Result<OrderbookUpdateStream, Error> {
        let (orderbook_update_sink, orderbook_update_stream) = futures::channel::mpsc::unbounded();
        self.orderbook_update_sinks
            .lock()
            .await
            .entry(pair.to_string())
            .or_default()
            .push(orderbook_update_sink);

        Ok(orderbook_update_stream.boxed())
    }
}
