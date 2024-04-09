use std::collections::{HashMap, HashSet};

use alloy_primitives::Address;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HydranetConfig {
    pub lssd_url: String,
    pub lnd_url: String,
    pub lnd_cert_dir: String,
    pub lnd_macaroon_dir: String,
    pub lnd_service_url: String,
    pub connext_url: String,
    pub connext_service_url: String,
    pub connext_event_resolver_url: String,
    pub connext_vector_id: String,
    pub evm_networks: HashMap<EvmNetwork, ConnextConfig>,
    pub assets: HashMap<String, Asset>,
    pub pairs: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvmNetwork {
    Ethereum,
    Arbitrum,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct ConnextConfig {
    pub connext_contract: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(tag = "protocol", rename_all = "snake_case")]
pub enum Asset {
    Utxo {
        decimals: u8,
    },
    Evm {
        network: EvmNetwork,
        address: Address,
        decimals: u8,
    },
}

impl Asset {
    pub fn decimals(&self) -> u8 {
        match self {
            Asset::Utxo { decimals } => *decimals,
            Asset::Evm { decimals, .. } => *decimals,
        }
    }
}
