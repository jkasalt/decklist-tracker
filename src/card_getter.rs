use anyhow::{anyhow, Context, Result};
use indicatif::ProgressBar;
use reqwest::Url;
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

    pub fn owned_cards(translator: &mut MtgaIdTranslator) -> Result<Collection> {
        let to_parse = format!("{}/cards", Self::address());
        let url =
            Url::parse(&to_parse).with_context(|| anyhow!("Failed to parse url {to_parse}"))?;
        let response = reqwest::blocking::get(url)
            .context(
                "Unable to get cards from daemon. Are you sure the daemon is running on port 9000?",
            )?
            .json()
            .context("Unable to parse json from card daemon. Make sure the game is idling in the main menu.")?;
        let cards: Vec<NameAmount> = serde_json::from_value::<CardReply>(response)?.cards;
        let pb = ProgressBar::new(cards.len() as u64);
        let collection = cards
            .into_iter()
            .flat_map(|NameAmount { id, owned }| {
                pb.inc(1);
                let card_data = translator
                    .translate(id)?
                    .ok_or(anyhow!("Card with id {id} does not exist on scryfall"))?;
                Ok::<_, anyhow::Error>((card_data.name, owned, card_data.rarity, card_data.set))
            })
            .collect::<Collection>();
        pb.finish();
        Ok(collection)
    }

    pub fn fetch_card(name: impl AsRef<str>) -> Result<NetCardData> {
        let to_parse = format!(
            "https://api.scryfall.com/cards/named?exact={}",
            name.as_ref()
        );
        let url =
            Url::parse(&to_parse).with_context(|| anyhow!("Failed to parse url {to_parse}"))?;
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
