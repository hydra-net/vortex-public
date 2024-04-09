use anyhow::Error;
use async_trait::async_trait;
use rust_decimal::Decimal;
use vortex_core::api::InfoFetcher;
use vortex_core::types::Info;

use crate::Hydranet;

// TODO: implement info in lssd
// TODO: fees + min amounts

#[async_trait]
impl InfoFetcher for Hydranet {
    async fn get_market_info(&self, pair: &str) -> Result<Info, Error> {
        get_market_info(pair)
    }
}

pub fn get_market_info(pair: &str) -> Result<Info, Error> {
    let (base, quote) = pair.split_once('_').ok_or(Error::msg("invalid pair"))?;

    Ok(Info {
        pair: pair.to_string(),
        base: base.to_string(),
        quote: quote.to_string(),
        min_amount: Decimal::ZERO,
        min_usd_amount: Decimal::ZERO,
        amount_precision: 8,
        price_precision: 8,
        maker_fee: Decimal::ZERO,
        taker_fee: Decimal::ZERO,
    })
}
