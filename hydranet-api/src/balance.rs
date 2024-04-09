use std::collections::HashMap;

use alloy_primitives::U256;
use anyhow::Error;
use async_trait::async_trait;
use rust_decimal::Decimal;
use vortex_core::api::{BalanceFetcher, OrderFetcher};
use vortex_core::types::{Balance, BalanceUpdate, BalanceUpdateStream};

use crate::config::Asset;
use crate::utils::u256_to_decimal;
use crate::Hydranet;

#[async_trait]
impl BalanceFetcher for Hydranet {
    async fn get_balances(&self) -> Result<HashMap<String, Balance>, Error> {
        let mut offchain_balance = HashMap::new();

        for (coin, asset) in self.config.assets.clone() {
            let (can_send, can_receive) = match asset {
                Asset::Utxo { decimals } => {
                    let mut lnd_client = self.lnd_client.lock().await;
                    let balance = lnd_client.get_offchain_balance().await?;

                    (
                        u256_to_decimal(
                            U256::from(balance.local_balance.unwrap_or_default().sat),
                            decimals,
                        ),
                        u256_to_decimal(
                            U256::from(balance.remote_balance.unwrap_or_default().sat),
                            decimals,
                        ),
                    )
                }
                Asset::Evm {
                    network,
                    address,
                    decimals,
                } => {
                    let connext_config = self
                        .config
                        .evm_networks
                        .get(&network)
                        .ok_or(Error::msg("EVM network not configured"))?;

                    let (can_send, can_receive) = self
                        .connext_client
                        .get_coin_balance(
                            &self.config.connext_vector_id,
                            &connext_config.connext_contract,
                            &address,
                        )
                        .await?;

                    (
                        u256_to_decimal(can_send, decimals),
                        u256_to_decimal(can_receive, decimals),
                    )
                }
            };

            offchain_balance.insert(
                coin,
                Balance {
                    free: can_send,
                    used: Decimal::ZERO,
                    total: can_send,
                    remote_free: can_receive,
                    remote_used: Decimal::ZERO,
                    remote_total: can_receive,
                },
            );
        }

        for pair in &self.config.pairs {
            let pending_orders = self.get_orders_pending(pair).await?;
            for coin in self.config.assets.keys() {
                if pair.contains(&format!("{coin}_")) {
                    // coin is base currency
                    for ask in &pending_orders.sells {
                        if let Some(balance) = offchain_balance.get_mut(coin) {
                            balance.free = balance.free.saturating_sub(ask.amount);
                            balance.used = balance.used.saturating_add(ask.amount);
                        }
                    }
                    for bid in &pending_orders.buys {
                        if let Some(balance) = offchain_balance.get_mut(coin) {
                            balance.remote_free = balance.remote_free.saturating_sub(bid.amount);
                            balance.remote_used = balance.remote_used.saturating_add(bid.amount);
                        }
                    }
                } else if pair.contains(&format!("_{coin}")) {
                    // coin is quote currency
                    for bid in &pending_orders.buys {
                        if let Some(balance) = offchain_balance.get_mut(coin) {
                            balance.free = balance.free.saturating_sub(bid.amount * bid.price);
                            balance.used = balance.used.saturating_add(bid.amount * bid.price);
                        }
                    }
                    for ask in &pending_orders.sells {
                        if let Some(balance) = offchain_balance.get_mut(coin) {
                            balance.remote_free =
                                balance.remote_free.saturating_sub(ask.amount * ask.price);
                            balance.remote_used =
                                balance.remote_used.saturating_add(ask.amount * ask.price);
                        }
                    }
                }
            }
        }

        Ok(offchain_balance)
    }

    async fn subscribe_balances(&self) -> Result<BalanceUpdateStream, Error> {
        // TODO: implement polling
        let (sender, receiver) = futures::channel::mpsc::unbounded::<BalanceUpdate>();

        // NOTE: this is a hack to keep the stream alive (sender is never dropped)
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(u64::MAX)).await;
                sender.close_channel();
            }
        });

        Ok(Box::pin(receiver))
    }
}
