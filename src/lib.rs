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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Mythic,
    Land,
    Unknown,
}

pub struct CardData {
    pub amount: u8,
    pub name: String,
    pub rarity: Rarity,
}

// pub struct Missing {
//     missing_main: Vec<CardData>,
//     missing_side: Vec<CardData>,
// }

pub struct Collection {
    amounts: Vec<u8>,
    names: Vec<String>,
    rarities: Vec<Rarity>,
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
                    "common" => Rarity::Common,
                    "uncommon" => Rarity::Uncommon,
                    "rare" => Rarity::Rare,
                    "mythic" => Rarity::Mythic,
                    "land" => Rarity::Land,
                    _ => Rarity::Unknown,
                })
                .with_context(err_message)?;
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

    pub fn missing<'a>(&'a self, deck: &'a Deck) -> impl Iterator<Item = CardData> + 'a {
        deck.amounts_main
            .iter()
            .zip(deck.names_main.iter())
            .filter(|(_, deck_card_name)| {
                // Ignore basic lands
                !matches!(
                    deck_card_name.as_str(),
                    "Plains" | "Island" | "Swamp" | "Mountain" | "Forest"
                )
            })
            .chain(deck.amounts_side.iter().zip(deck.names_side.iter()))
            .map(|(n, name)| {
                // For each card in the deck
                self.names
                    .iter()
                    .position(|col_name| col_name == name)
                    .map_or_else(
                        || CardData {
                            amount: *n,
                            name: name.clone(),
                            rarity: Rarity::Unknown,
                        },
                        |i| {
                            let in_collection = self.amounts[i];
                            let amount_missing = n.saturating_sub(in_collection);
                            CardData {
                                amount: amount_missing,
                                name: name.clone(),
                                rarity: self.rarities[i],
                            }
                        },
                    )
            })
    }

    pub fn into_hash_map(self) -> HashMap<String, (u8, Rarity)> {
        let mut result = HashMap::with_capacity(self.names.len());
        for (i, name) in self.names.into_iter().enumerate() {
            let (other_amount, other_rarity) = (self.amounts[i], self.rarities[i]);
            let (ref mut cur_amount, ref mut cur_rarity) =
                result.entry(name).or_insert((other_amount, other_rarity));
            if *cur_amount < other_amount {
                *cur_amount = other_amount;
                *cur_rarity = other_rarity;
            }
        }
        result
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

impl std::fmt::Display for Deck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(companion) = self.companion.as_ref() {
            writeln!(f, "Companion\n1 {companion}\n")?;
        }
        writeln!(f, "Deck")?;
        for (amount, name) in self.amounts_main.iter().zip(self.names_main.iter()) {
            writeln!(f, "{amount} {name}")?;
        }
        if !self.names_side.is_empty() {
            writeln!(f, "\nSideboard")?;
            for (amount, name) in self.amounts_side.iter().zip(self.names_side.iter()) {
                writeln!(f, "{amount} {name}")?;
            }
        }
        Ok(())
    }
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
            let name: String =
                Itertools::intersperse(words.take_while(|w| !w.starts_with('(')), " ").collect();
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
    pub fn iter_mut(&mut self) -> std::slice::IterMut<Deck> {
        self.decks.iter_mut()
    }
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
