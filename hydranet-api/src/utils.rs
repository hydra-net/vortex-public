use std::str::FromStr;

use alloy_primitives::U256;
use anyhow::Error;
use rust_decimal::Decimal;
use vortex_core::types::{Side, TradeOrder, Type};

use crate::protos;

const MAX_DECIMAL_PLACES: u8 = 77; // (2 ** 256) - 1 ≈ 10 ** 77

pub fn u256_to_decimal(value: U256, decimals: u8) -> Decimal {
    if decimals <= MAX_DECIMAL_PLACES {
        let mut value_str = value.to_string();
        let value_len = value_str.len();

        if value_len > decimals as usize {
            value_str.insert(value_len - decimals as usize, '.');
        } else {
            value_str.insert(0, '0');
            value_str.insert(1, '.');
            for i in 0..(decimals as usize - value_len) {
                value_str.insert(2 + i, '0');
            }
        }

        match Decimal::from_str_exact(&value_str) {
            Ok(dec_value) => dec_value,
            Err(rust_decimal::Error::Underflow) => Decimal::ZERO,
            Err(_) => Decimal::MAX,
        }
    } else {
        Decimal::ZERO
    }
}

// pub fn decimal_to_u256(value: Decimal, decimals: u8) -> U256 {
//     if decimals <= MAX_DECIMAL_PLACES {
//         let value_str = value.to_string();

//         let (int_part, dec_part) = value_str.split_once('.').unwrap_or((&value_str, ""));
//         let dec_part_len = dec_part.len();

//         let mut int_part = int_part.to_string();
//         let mut dec_part = dec_part.to_string();

//         let amount_str = match dec_part_len.cmp(&(decimals as usize)) {
//             std::cmp::Ordering::Greater => {
//                 int_part.truncate(value_str.len() - (dec_part_len - decimals as usize));
//                 int_part
//             }
//             std::cmp::Ordering::Less => {
//                 for _ in 0..(decimals as usize - dec_part_len) {
//                     dec_part.push('0');
//                 }

//                 int_part.to_string() + &dec_part
//             }
//             std::cmp::Ordering::Equal => int_part.to_string() + &dec_part,
//         };

//         U256::from_str(&amount_str).unwrap()
//     } else {
//         U256::ZERO
//     }
// }

impl From<Decimal> for protos::BigInteger {
    fn from(value: Decimal) -> Self {
        protos::BigInteger {
            value: value.to_string(),
        }
    }
}

impl TryFrom<protos::BigInteger> for Decimal {
    type Error = Error;

    fn try_from(value: protos::BigInteger) -> Result<Self, Self::Error> {
        Decimal::from_str(&value.value).map_err(Error::from)
    }
}

pub fn proto_order_to_trade_order(order: &protos::Order) -> Result<TradeOrder, Error> {
    let amount = Decimal::try_from(
        order
            .funds
            .as_ref()
            .ok_or(Error::msg("Funds not found"))?
            .clone(),
    )?;

    let mut executed = Decimal::ZERO;
    for trade in &order.closed {
        executed += Decimal::try_from(
            trade
                .amount
                .as_ref()
                .ok_or(Error::msg("Funds not found"))?
                .clone(),
        )?;
    }
    let left = amount - executed;
    let side = if order.side == protos::OrderSide::Sell as i32 {
        Side::Sell
    } else {
        Side::Buy
    };

    Ok(TradeOrder {
        id: order.order_id.clone(),
        timestamp_millis: order.created_at * 1000,
        side,
        r#type: Type::Limit,
        price: order
            .price
            .as_ref()
            .ok_or(Error::msg("Price not found"))?
            .clone()
            .try_into()?,
        amount,
        left,
        executed,
    })
}
