use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::Exchange;

use crate::strategies::{
    arbitrage::ArbitrageConfig, grid::GridConfig, volume_maker::VolumeMakerConfig,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BotConfig {
    pub settings: Settings,
    pub strategies: Vec<Strategy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct Settings {
    pub cancel_orders_on_start: bool,
    pub cancel_orders_on_exit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    Grid {
        exchange: Exchange,
        pair: String,
        #[serde(flatten)]
        config: GridConfig,
    },
    VolumeMaker {
        exchange: Exchange,
        pair: String,
        #[serde(flatten)]
        config: VolumeMakerConfig,
    },
    Arbitrage {
        markets: Vec<ExchangePair>,
        #[serde(flatten)]
        config: ArbitrageConfig,
    },
}

impl Strategy {
    pub fn markets(&self) -> HashMap<Exchange, HashSet<String>> {
        match self {
            Self::Grid { exchange, pair, .. } => {
                let mut map = HashMap::new();
                map.entry(*exchange)
                    .or_insert_with(HashSet::new)
                    .insert(pair.clone());
                map
            }
            Self::VolumeMaker { exchange, pair, .. } => {
                let mut map = HashMap::new();
                map.entry(*exchange)
                    .or_insert_with(HashSet::new)
                    .insert(pair.clone());
                map
            }
            Self::Arbitrage { markets, .. } => {
                let mut map = HashMap::new();
                for market in markets {
                    map.entry(market.exchange)
                        .or_insert_with(HashSet::new)
                        .insert(market.pair.clone());
                }
                map
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExchangePair {
    pub exchange: Exchange,
    pub pair: String,
}
