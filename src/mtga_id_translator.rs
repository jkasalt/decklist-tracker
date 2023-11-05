use anyhow::{anyhow, bail, Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::Rarity;

const SCRYFALL_WAIT_TIME: Duration = Duration::from_millis(75);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct NetCardData {
    pub name: String,
    pub rarity: Rarity,
    pub set: String,
}

pub struct MtgaIdTranslator {
    cache: HashMap<u32, Option<NetCardData>>,
    last_request_time: Option<Instant>,
    repository: PathBuf,
}

impl MtgaIdTranslator {
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self> {
        if !path.as_ref().exists() {
            std::fs::write(&path, "{}")?;
        }
        let file = File::open(&path)?;
        let cache = ron::de::from_reader(&file)?;
        Ok(MtgaIdTranslator {
            cache,
            last_request_time: None,
            repository: path.as_ref().to_path_buf(),
        })
    }

    #[inline]
    fn handle_wait(&mut self) {
        if let Some(time) = self.last_request_time {
            let wait_time = SCRYFALL_WAIT_TIME.saturating_sub(Instant::now() - time);
            std::thread::sleep(wait_time);
        }
        self.last_request_time = Some(Instant::now());
    }

    pub fn translate(&mut self, id: u32) -> Result<Option<NetCardData>> {
        if let Some(card_data) = self.cache.get(&id) {
            return Ok(card_data.clone());
        }
        self.handle_wait();
        let response =
            reqwest::blocking::get(format!("https://api.scryfall.com/cards/arena/{id}"))?;
        if response.status() == StatusCode::NOT_FOUND {
            self.cache.insert(id, None);
        }
        if response.status() != StatusCode::OK {
            bail!(
                "requesting card with id {id} from scryfall failed with status {}",
                response.status()
            );
        }
        let card_data: NetCardData = response
            .json()
            .with_context(|| anyhow!("Failed to parse card with arena id {id}"))?;
        self.cache.insert(id, Some(card_data.clone()));
        Ok(Some(card_data))
    }

    fn write(&self) -> Result<()> {
        std::fs::write(&self.repository, ron::to_string(&self.cache)?)?;
        Ok(())
    }
}

impl Drop for MtgaIdTranslator {
    fn drop(&mut self) {
        self.write()
            .unwrap_or_else(|err| eprintln!("ERROR: while closing translator, {err}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::NamedTempFile;

    #[test]
    fn translate() -> Result<()> {
        let dictionary = NamedTempFile::new("dictionary.json")?;
        let mut translator = MtgaIdTranslator::load_from_file(dictionary)?;
        let reply = translator.translate(75310)?;
        assert_eq!(
            reply,
            Some(NetCardData {
                name: "Hengegate Pathway // Mistgate Pathway".to_owned(),
                rarity: Rarity::Rare,
                set: "khm".to_owned(),
            })
        );
        Ok(())
    }
}
