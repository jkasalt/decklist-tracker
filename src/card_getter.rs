use anyhow::{anyhow, Context, Result};
use indicatif::ProgressBar;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::Value;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::process::{Child, Command};

use crate::mtga_id_translator::{MtgaIdTranslator, NetCardData};
use crate::Collection;

#[derive(Deserialize)]
struct NameAmount {
    #[serde(rename = "grpId")]
    id: u32,
    owned: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardReply {
    cards: Vec<NameAmount>,
    elapsed_time: u32,
}

pub struct CardGetter;

impl CardGetter {
    fn address() -> &'static str {
        "http://localhost:9000"
    }

    pub fn owned_cards<P: AsRef<Path>>(translator: &mut MtgaIdTranslator<P>) -> Result<Collection> {
        let url = format!("{}/cards", Self::address());
        let response = reqwest::blocking::get(url)
            .context(
                "Unable to get cards from daemon. Are you sure the daemon is running on port 9000?",
            )?
            .json()
            .context("Unable to parse json from card daemon. Are you sure the game is running?")?;
        let cards: Vec<NameAmount> = serde_json::from_value::<CardReply>(response)?.cards;
        let mut names = Vec::with_capacity(cards.len());
        let mut amounts = Vec::with_capacity(cards.len());
        let mut rarities = Vec::with_capacity(cards.len());
        let mut sets = Vec::with_capacity(cards.len());
        let pb = ProgressBar::new(cards.len() as u64);
        for NameAmount { id, owned } in cards {
            pb.inc(1);
            let net_card_data = match translator.translate(id) {
                Ok(net_card_data) => net_card_data,
                Err(_) => {
                    // pb.println(format!("Failed to translate card: {e}"));
                    continue;
                }
            };
            names.push(net_card_data.name);
            amounts.push(owned);
            rarities.push(net_card_data.rarity);
            sets.push(net_card_data.set);
        }
        pb.finish();
        Ok(Collection {
            names,
            amounts,
            rarities,
            sets,
        })
    }

    pub fn fetch_card(name: impl AsRef<str>) -> Result<NetCardData> {
        let url = format!(
            "https://api.scryfall.com/cards/named?exact={}",
            name.as_ref()
        );
        let response = reqwest::blocking::get(url)
            .with_context(|| anyhow!("Unable to find {} on scryfall", name.as_ref()))?
            .json()
            .context("Unable to parse json from Scryfall")?;
        let net_card_data = serde_json::from_value(response)?;
        Ok(net_card_data)
    }

    #[cfg(test)]
    pub fn status() -> Result<Value> {
        let url = format!("{}/status", Self::address());
        let response = reqwest::blocking::get(url)?.text()?;
        let value = serde_json::from_str(response.as_str())?;
        Ok(value)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn get_status() {
        let status = CardGetter::status().unwrap();
        let status = status.as_object().unwrap();
        assert!(status.len() == 4);
        assert!(status.contains_key("isRunning"));
        assert!(status.contains_key("daemonVersion"));
        assert!(status.contains_key("updating"));
        assert!(status.contains_key("processId"));
    }
}
