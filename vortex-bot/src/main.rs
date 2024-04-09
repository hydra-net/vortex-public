//#![feature(never_type)]

use std::fmt::Display;
use std::{collections::HashMap, sync::Arc};

use ::config::{Config, File};
use anyhow::Error;
use config::Settings;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use hydranet_api::{Hydranet, HydranetConfig};

use serde::Deserialize;
use strategies::arbitrage::start_arbitrage_strategy;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use tracing_subscriber::{prelude::*, EnvFilter};
use vortex_core::exchange::ExchangeClient;
use vortex_core::market::Market;


mod config;
mod strategies;

use crate::strategies::grid::start_grid_strategy;
use crate::strategies::volume_maker::start_volume_maker_strategy;

use self::config::{BotConfig, Strategy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exchange {
    
    
    Hydranet,
    
}

impl Display for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Exchange::Hydranet => write!(f, "hydranet"),
        }
    }
}

struct Bot {
    settings: Settings,
    exchanges: HashMap<Exchange, Arc<ExchangeClient>>,
    markets: HashMap<Exchange, HashMap<String, Arc<Market>>>, // exchange -> pair -> market
    strategies: HashMap<Strategy, Vec<JoinHandle<()>>>,        // strategy -> strategy_task_handles
}

impl Bot {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            exchanges: HashMap::new(),
            markets: HashMap::new(),
            strategies: HashMap::new(),
        }
    }

    pub async fn run_strategy(&mut self, strategy: Strategy) -> Result<(), Error> {
        // init exchange and markets
        for (exchange, pairs) in strategy.markets() {
            let exchange_client = if let Some(exchange_client) = self.exchanges.get(&exchange) {
                exchange_client.clone()
            } else {
                match exchange {
             
                     
                    Exchange::Hydranet => {
                        let config = Config::builder()
                            .add_source(File::with_name(&std::env::var("HYDRANET_CONFIG_FILE")?))
                            .build()?;

                        let config: HydranetConfig = config.try_deserialize()?;

                        let client = Hydranet::new(config).await?;
                        let exchange_client = Arc::new(ExchangeClient::new(client).await);
                        self.exchanges
                            .insert(Exchange::Hydranet, exchange_client.clone());

                        exchange_client
                    }
           
                }
                
            };

            for pair in pairs {
                let market_entry = self.markets.entry(exchange).or_default();
                if let std::collections::hash_map::Entry::Vacant(e) =
                    market_entry.entry(pair.clone())
                {
                    let market = Market::new(
                        exchange_client.clone(),
                        &pair,
                        self.settings.cancel_orders_on_start,
                    )
                    .await?;
                    e.insert(Arc::new(market));
                }
            }
        }

        let task_handles = match strategy.clone() {
            Strategy::Grid {
                exchange,
                pair,
                config,
            } => {
                let market = self.markets.get(&exchange).unwrap().get(&pair).unwrap();

                vec![start_grid_strategy(market.clone(), config).await?]
            }
            Strategy::VolumeMaker {
                exchange,
                pair,
                config,
            } => {
                let market = self.markets.get(&exchange).unwrap().get(&pair).unwrap();

                vec![start_volume_maker_strategy(market.clone(), config).await?]
            }
            Strategy::Arbitrage { markets, config } => {
                let markets = markets
                    .into_iter()
                    .map(|exchange_pair| {
                        self.markets
                            .get(&exchange_pair.exchange)
                            .unwrap()
                            .get(&exchange_pair.pair)
                            .unwrap()
                            .clone()
                    })
                    .collect::<Vec<_>>();

                start_arbitrage_strategy(markets, config).await?
            }
        };

        self.strategies.insert(strategy, task_handles);

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenv::dotenv().ok();

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(
            tracing_subscriber::fmt::layer()
                .with_line_number(true)
                .compact(),
        )
        .init();

    let config = Config::builder()
        .add_source(File::with_name("config"))
        .build()?;

    let config: BotConfig = config.try_deserialize()?;

    let mut bot = Bot::new(config.settings);

    for strategy in config.strategies {
        bot.run_strategy(strategy).await?;
    }

    tokio::signal::ctrl_c().await?;

    // stop all strategies on exit
    bot.strategies.into_values().for_each(|task_handles| {
        task_handles.into_iter().for_each(|task_handle| {
            task_handle.abort();
        });
    });

    // cancel all orders on exit
    if bot.settings.cancel_orders_on_exit {
        info!("Canceling all orders on exit...");

        let cancel_orders_futures = FuturesUnordered::new();

        for (exchange, markets) in bot.markets.iter() {
            for market in markets.values() {
                cancel_orders_futures.push(
                    async move {
                        if let Err(e) = market.cancel_all_orders().await {
                            warn!("Failed to cancel all orders on {}: {}", exchange, e);
                        }
                    }
                    .boxed(),
                );
            }
        }

        futures::future::join_all(cancel_orders_futures).await;
    }

    info!("Exiting...");

    Ok(())
}
