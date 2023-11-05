use crate::mtga_id_translator::{MtgaIdTranslator, NetCardData};
use crate::{Collection, Rarity};
use anyhow::{anyhow, Context, Result};
use indicatif::ProgressBar;
use reqwest::Url;
use serde::Deserialize;

#[derive(Deserialize)]
struct NameAmount {
    #[serde(rename = "grpId")]
    id: u32,
    owned: u8,
}

#[derive(Deserialize)]
struct ScryfallReply {
    data: Vec<ScryfallCardData>,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ScryfallGames {
    Paper,
    Mtgo,
    Arena,
}

#[derive(Deserialize)]
struct ScryfallCardData {
    name: String,
    games: Vec<ScryfallGames>,
    rarity: Rarity,
    set: String,
}

#[derive(Deserialize)]
struct DaemonReply {
    cards: Vec<NameAmount>,
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
        let cards: Vec<NameAmount> = reqwest::blocking::get(url)
            .context(
                "Unable to get cards from daemon. Are you sure the daemon is running on port 9000?",
            )?
            .json::<DaemonReply>()
            .context("Unable to parse json from card daemon. Make sure the game is idling in the main menu.")?.cards;
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
        pb.finish_and_clear();
        Ok(collection)
    }

    pub fn fetch_card(name: impl AsRef<str>) -> Result<Vec<NetCardData>> {
        let name = crate::collection::simplified_name(&name);
        let to_parse = format!(
            "https://api.scryfall.com/cards/search?q={}&unique=prints",
            name
        );
        let url =
            Url::parse(&to_parse).with_context(|| anyhow!("Failed to parse url {to_parse}"))?;
        let prints: Vec<ScryfallCardData> = reqwest::blocking::get(url)
            .with_context(|| anyhow!("Unable to find {} on scryfall", name))?
            .json::<ScryfallReply>()
            .map_err(|err| anyhow!("Unable to parse json from Scryfall: {err}"))?
            .data;
        let relevant = prints
            .into_iter()
            .filter(|print| print.games.contains(&ScryfallGames::Arena))
            .map(
                |ScryfallCardData {
                     name,
                     rarity,
                     set,
                     games: _,
                 }| NetCardData { name, rarity, set },
            )
            .collect();
        Ok(relevant)
    }

    #[cfg(test)]
    pub fn status() -> Result<serde_json::Value> {
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
