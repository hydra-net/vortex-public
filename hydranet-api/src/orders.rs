use std::time::Duration;

use anyhow::Error;
use async_trait::async_trait;
use rust_decimal::Decimal;
use vortex_core::api::{BalanceFetcher, CreateOrderError, OrderCreator, OrderFetcher};
use vortex_core::types::{Side, TradeOrder, Type};

use crate::{protos, PairOrderId};
use crate::utils::proto_order_to_trade_order;
use crate::Hydranet;

const ORDER_TIMEOUT: Duration = Duration::from_secs(90);

#[async_trait]
impl OrderCreator for Hydranet {
    async fn create_limit_order(
        &self,
        pair: &str,
        side: Side,
        amount: Decimal,
        price: Decimal,
    ) -> Result<TradeOrder, CreateOrderError> {
        let (base, quote) = pair
            .split_once('_')
            .ok_or(Error::msg("Invalid pair"))
            .map_err(CreateOrderError::Other)?;
        let balances = self.get_balances().await.map_err(CreateOrderError::Other)?;
        let base_balance = balances
            .get(base)
            .ok_or(Error::msg("Base balance not found"))
            .map_err(CreateOrderError::Other)?;
        let quote_balance = balances
            .get(quote)
            .ok_or(Error::msg("Quote balance not found"))
            .map_err(CreateOrderError::Other)?;

        let funds = match side {
            Side::Sell => {
                if base_balance.free < amount || quote_balance.remote_free < amount * price {
                    return Err(CreateOrderError::InsufficientFunds)?;
                }

                protos::BigInteger::from(amount)
            }
            Side::Buy => {
                if quote_balance.free < amount * price || base_balance.remote_free < amount {
                    return Err(CreateOrderError::InsufficientFunds)?;
                }

                // TODO: maybe implement limit buy with base coin in lssd
                protos::BigInteger::from(amount * price)
            }
        };

        let request_msg = protos::PlaceOrderRequest {
            pair_id: pair.to_string(),
            side: side as i32,
            funds: Some(funds),
            price: Some(protos::BigInteger::from(price)),
        };

        let mut request = tonic::Request::new(request_msg.clone());
        request.set_timeout(ORDER_TIMEOUT);

        let mut orders_client = self.orders_client.lock().await;
        let order_response = orders_client
            .place_order(request)
            .await
            .map_err(|e| CreateOrderError::Other(e.into()))?
            .into_inner();

        match order_response.outcome.unwrap() {
            protos::place_order_response::Outcome::SwapSuccess(successful_order) => {
                let timestamp_millis = std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                return Ok(TradeOrder {
                    id: successful_order.order_id,
                    timestamp_millis,
                    side,
                    r#type: Type::Limit,
                    price: match successful_order.price {
                        Some(price) => price.try_into().map_err(CreateOrderError::Other)?,
                        None => price,
                    },
                    amount,
                    left: Decimal::ZERO,
                    executed: amount,
                });
            }
            protos::place_order_response::Outcome::Order(order) => {
                proto_order_to_trade_order(&order).map_err(CreateOrderError::Other)
            }
            protos::place_order_response::Outcome::Failure(failure) => match failure.failure {
                Some(protos::place_order_failure::Failure::OrderbookFailure(failure)) => {
                    return Err(CreateOrderError::Other(Error::msg(failure.failure_reason)))?
                }
                Some(protos::place_order_failure::Failure::SwapFailure(failure)) => {
                    return Err(CreateOrderError::Other(Error::msg(failure.failure_reason)))?
                }
                None => return Err(CreateOrderError::Other(Error::msg("Unknown failure")))?,
            },
        }
    }

    async fn create_market_order(
        &self,
        _pair: &str,
        _side: Side,
        _amount: Decimal,
    ) -> Result<TradeOrder, CreateOrderError> {
        // TODO: implement market order (would be better to implement market buy with base coin in lssd first)
        unimplemented!()
    }

    async fn create_hold_market_order(
        &self,
        pair: &str,
        side: Side,
        amount: Decimal,
    ) -> Result<TradeOrder, CreateOrderError> {
        let funds = protos::BigInteger::from(amount);

        let request_msg = protos::PlaceOrderRequest {
            pair_id: pair.to_string(),
            side: match side {
                Side::Sell => protos::OrderSide::Sell as i32,
                Side::Buy => protos::OrderSide::Buy as i32,
            },
            funds: Some(funds),
            price: Some(protos::BigInteger {
                value: "0".to_string(),
            }),
        };

        let mut request = tonic::Request::new(request_msg.clone());
        request.set_timeout(ORDER_TIMEOUT);

        let mut orders_client = self.orders_client.lock().await;
        let order_response = orders_client
            .place_order(request)
            .await
            .map_err(|e| CreateOrderError::Other(e.into()))?
            .into_inner();

        match order_response.outcome.unwrap() {
            protos::place_order_response::Outcome::SwapSuccess(successful_order) => {
                let timestamp_millis = std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                let price: Decimal = successful_order
                    .price
                    .ok_or(Error::msg("Price not found"))
                    .map_err(CreateOrderError::Other)?
                    .try_into()
                    .map_err(CreateOrderError::Other)?;
                return Ok(TradeOrder {
                    id: successful_order.order_id,
                    timestamp_millis,
                    side,
                    r#type: Type::Limit,
                    price,
                    amount,
                    left: Decimal::ZERO,
                    executed: amount,
                });
            }
            protos::place_order_response::Outcome::Order(order) => {
                proto_order_to_trade_order(&order).map_err(CreateOrderError::Other)
            }
            protos::place_order_response::Outcome::Failure(failure) => match failure.failure {
                Some(protos::place_order_failure::Failure::OrderbookFailure(failure)) => {
                    return Err(CreateOrderError::Other(Error::msg(failure.failure_reason)))?
                }
                Some(protos::place_order_failure::Failure::SwapFailure(failure)) => {
                    return Err(CreateOrderError::Other(Error::msg(failure.failure_reason)))?
                }
                None => return Err(CreateOrderError::Other(Error::msg("Unknown failure")))?,
            },
        }
    }

    async fn cancel_order(&self, pair: &str, id: &str) -> Result<(), CreateOrderError> {
        let pair_order_id = PairOrderId {
            pair: pair.to_string(),
            order_id: id.to_string(),
        };
        let latest_order_id = self.order_id_map.lock().await.get(&pair_order_id).cloned().unwrap_or(id.to_string());
        let request = protos::CancelOrderRequest {
            pair_id: pair.to_string(),
            order_id: latest_order_id   
        };

        let mut orders_client = self.orders_client.lock().await;
        orders_client
            .cancel_order(request.clone())
            .await
            .map_err(|e| CreateOrderError::Other(e.into()))?;

        Ok(())
    }

    async fn cancel_all_orders(&self, pair: &str) -> Result<(), CreateOrderError> {
        let orders = self
            .get_orders_pending(pair)
            .await
            .map_err(CreateOrderError::Other)?;
        for order in orders.buys {
            self.cancel_order(pair, &order.id).await?;
        }
        for order in orders.sells {
            self.cancel_order(pair, &order.id).await?;
        }
        Ok(())
    }
}
