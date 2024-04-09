use alloy_primitives::{Address, U256};
use anyhow::Error;
use serde_json::Value as JsonValue;
use std::str::FromStr;

pub struct ConnextClient {
    vector_url: String,
}

impl ConnextClient {
    pub fn new(vector_url: &str) -> Self {
        Self {
            vector_url: vector_url.to_string(),
        }
    }

    pub async fn get_coin_balance(
        &self,
        vector_id: &str,
        channel_id: &Address,
        asset_id: &Address,
    ) -> Result<(U256, U256), Error> {
        let channel = reqwest::get(format!(
            "{}/{}/channels/{}",
            self.vector_url, vector_id, channel_id
        ))
        .await?
        .text()
        .await?;

        let channel_json = JsonValue::from_str(&channel)?;

        let assets = channel_json["assetIds"]
            .as_array()
            .ok_or(Error::msg("Could not get assets from channel json"))?;

        let mut asset_index = None;
        for (i, asset) in assets.iter().enumerate() {
            let asset_str = asset
                .as_str()
                .ok_or(Error::msg("Could not get asset string"))?;

            if &Address::from_str(asset_str)? == asset_id {
                asset_index = Some(i);
                break;
            }
        }

        let asset_index = asset_index.ok_or(Error::msg("Could not find asset index"))?;

        let balances = channel_json["balances"]
            .as_array()
            .ok_or(Error::msg("Could not get balances from channel json"))?;

        let can_send_balance = balances
            .get(asset_index)
            .ok_or(Error::msg("Could not get can send balance"))?
            .get("amount")
            .ok_or(Error::msg("Could not get can send balance amount"))?
            .get(1)
            .ok_or(Error::msg("Could not get can send balance amount 1"))?
            .as_str()
            .ok_or(Error::msg("Could not get can send balance amount 1 as str"))?;
        let can_receive_balance = balances
            .get(asset_index)
            .ok_or(Error::msg("Could not get can receive balance"))?
            .get("amount")
            .ok_or(Error::msg("Could not get can receive balance amount"))?
            .get(0)
            .ok_or(Error::msg("Could not get can receive balance amount 0"))?
            .as_str()
            .ok_or(Error::msg(
                "Could not get can receive balance amount 0 as str",
            ))?;

        Ok((
            U256::from_str(can_send_balance)?,
            U256::from_str(can_receive_balance)?,
        ))
    }
}
