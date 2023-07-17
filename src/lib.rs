use anyhow::{anyhow, bail, Context, Result};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::Path,
    str::FromStr,
};

pub struct Collection {
    amounts: Vec<u8>,
    names: Vec<String>,
    rarities: Vec<Rarity>,
}

#[derive(Debug, Clone, Copy)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Mythic,
    Land,
}

impl Collection {
    pub fn from_csv(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).context("Failed to find collection csv file")?;
        let num_lines = content.lines().count();
        let mut amounts = Vec::with_capacity(num_lines);
        let mut names = Vec::with_capacity(num_lines);
        let mut rarities = Vec::with_capacity(num_lines);

        for (i, line) in content.lines().enumerate().skip(1) {
            let err_message = || {
                format!("Failed to read line {line} (number {i}) in file {path:?}, as it is not in the expected format")
            };
            let mut elements = line.split(';');
            let amount = elements.next().with_context(err_message)?.parse()?;
            let name = elements.next().with_context(err_message)?.to_owned();
            let rarity = elements
                .nth(2)
                .map(|s| match s {
                    "common" => Ok(Rarity::Common),
                    "uncommon" => Ok(Rarity::Uncommon),
                    "rare" => Ok(Rarity::Rare),
                    "mythic" => Ok(Rarity::Mythic),
                    "land" => Ok(Rarity::Land),
                    x => bail!("Unexpected rarity `{x}`"),
                })
                .with_context(err_message)??;
            amounts.push(amount);
            names.push(name);
            rarities.push(rarity);
        }

        Ok(Collection {
            amounts,
            names,
            rarities,
        })
    }

    pub fn missing(&self, deck: &Deck) -> Result<(u8, u8, u8, u8)> {
        Ok(deck
            .amounts_main
            .iter()
            .zip(deck.names_main.iter())
            .map(|(n, name)| {
                self.names
                    .iter()
                    .position(|col_name| col_name == name)
                    .map(|i| {
                        let in_collection = self.amounts[i];
                        (n.saturating_sub(in_collection).max(0), self.rarities[i])
                    })
                    .map(|(m, r)| match r {
                        Rarity::Common => (m, 0, 0, 0),
                        Rarity::Uncommon => (0, m, 0, 0),
                        Rarity::Rare => (0, 0, m, 0),
                        Rarity::Mythic => (0, 0, 0, m),
                        Rarity::Land => (0, 0, 0, 0),
                    })
                    .ok_or(anyhow!("Card `{name}` is missing from the collection"))
            })
            .collect::<Result<Vec<_>>>()?
            .iter()
            .fold((0, 0, 0, 0), |acc, x| {
                (acc.0 + x.0, acc.1 + x.1, acc.2 + x.2, acc.3 + x.3)
            }))
    }

    pub fn into_hash_map(self) -> HashMap<String, (u8, Rarity)> {
        self.names
            .into_iter()
            .zip(self.amounts.into_iter().zip(self.rarities.into_iter()))
            .collect()
    }
}

impl std::fmt::Debug for Collection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.amounts
            .iter()
            .zip(self.names.iter())
            .zip(self.rarities.iter())
            .try_for_each(|((a, n), r)| writeln!(f, "{a} {n} ({r:?})"))?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Deck {
    pub name: String,
    companion: Option<String>,
    amounts_main: Vec<u8>,
    names_main: Vec<String>,
    amounts_side: Vec<u8>,
    names_side: Vec<String>,
}

impl FromStr for Deck {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        enum ParsingMode {
            Companion,
            Main,
            Side,
        }
        let mut parsing_mode = ParsingMode::Main;
        let mut amounts_main = Vec::new();
        let mut names_main = Vec::new();
        let mut amounts_side = Vec::new();
        let mut names_side = Vec::new();
        let mut companion = None;
        for (i, l) in s.lines().enumerate() {
            match l.trim() {
                "Companion" => {
                    parsing_mode = ParsingMode::Companion;
                    continue;
                }
                "Deck" => {
                    parsing_mode = ParsingMode::Main;
                    continue;
                }
                "Sideboard" => {
                    parsing_mode = ParsingMode::Side;
                    continue;
                }
                "" => {
                    continue;
                }
                _ => {}
            };
            let error_message = || {
                format!("Expected line {} to be of the form `{{integer}} {{card_name}},` but found `{l}`", i+1)
            };
            let mut words = l.split(' ');
            let num = words
                .next()
                .with_context(error_message)?
                .parse()
                .with_context(error_message)?;
            let name: String = words
                .take_while(|w| !w.starts_with('('))
                .intersperse(" ")
                .collect();
            let name = name.to_string();
            if name.is_empty() {
                bail!(error_message());
            }
            match parsing_mode {
                ParsingMode::Companion => companion = Some(name),
                ParsingMode::Main => {
                    amounts_main.push(num);
                    names_main.push(name);
                }
                ParsingMode::Side => {
                    amounts_side.push(num);
                    names_side.push(name);
                }
            }
        }
        Ok(Deck {
            name: "Unnamed".to_owned(),
            amounts_main,
            amounts_side,
            names_main,
            names_side,
            companion,
        })
    }
}

impl Deck {
    pub fn name(self, name: &str) -> Self {
        Deck {
            name: name.to_owned(),
            ..self
        }
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        fs::read_to_string(path)?.parse()
    }

    pub fn cards(&self) -> impl Iterator<Item = (&u8, &String)> {
        self.amounts_main
            .iter()
            .zip(self.names_main.iter())
            .chain(self.amounts_side.iter().zip(self.names_side.iter()))
    }
}

#[derive(Debug)]
pub struct Roster<P: AsRef<Path>> {
    path: P,
    decks: Vec<Deck>,
}

impl<P: AsRef<Path>> Roster<P> {
    pub fn iter(&self) -> std::slice::Iter<Deck> {
        self.decks.iter()
    }

    pub fn open(path: P) -> Result<Self> {
        if !path.as_ref().exists() {
            let mut file = File::create(&path)?;
            file.write_all(b"[]")?;
        }
        let decks = if !path.as_ref().exists() {
            Vec::new()
        } else {
            let file = File::open(&path)?;
            serde_json::from_reader(file)
                .map_err(|err| anyhow!("Failed to deserialize roster: {err}"))?
        };
        Ok(Roster { path, decks })
    }

    // TODO: change &Deck to Generic Cow<Deck>
    pub fn add_deck(&mut self, deck: &Deck) {
        self.decks.push(deck.clone());
    }

    pub fn remove_deck(&mut self, name: &str) -> Result<()> {
        let i = self
            .iter()
            .position(|deck| deck.name == name)
            .ok_or(anyhow!("The query `{name}` found no matching deck"))
            .context("Failed to remove deck")?;
        self.decks.swap_remove(i);
        Ok(())
    }

    pub fn write(&mut self) -> Result<()> {
        fs::write(&self.path, serde_json::to_string(&self.decks)?.as_bytes())?;
        Ok(())
    }

    pub fn deck_list(&self) -> impl Iterator<Item = &str> {
        self.iter().map(|deck| deck.name.as_str())
    }
}

impl<P: AsRef<Path>> Drop for Roster<P> {
    fn drop(&mut self) {
        self.write()
            .unwrap_or_else(|err| eprintln!("ERROR: while closing roster, {err}"));
    }
}
