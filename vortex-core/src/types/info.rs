use rust_decimal::Decimal;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Info {
    pub pair: String,
    pub base: String,
    pub quote: String,
    pub min_amount: Decimal,
    pub min_usd_amount: Decimal,
    pub amount_precision: u8,
    pub price_precision: u8,
    pub maker_fee: Decimal,
    pub taker_fee: Decimal,
}
