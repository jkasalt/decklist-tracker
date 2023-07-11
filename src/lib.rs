use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    str::FromStr,
};

pub const COLLECTION_PATH: &str =
    r"C:\Users\Luca Bracone\AppData\Roaming\my_projects\decklist_tracker\collection.csv";
pub const CATALOGUE_PATH: &str =
    r"C:\Users\Luca Bracone\AppData\Roaming\my_projects\decklist_tracker\catalogue.csv";

pub struct Collection {
    amounts: Vec<u32>,
    names: Vec<String>,
    rarities: Vec<u8>,
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
                    "common" => Ok(0),
                    "uncommon" => Ok(1),
                    "rare" => Ok(2),
                    "mythic" => Ok(3),
                    "land" => Ok(4),
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
}

impl std::fmt::Debug for Collection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.amounts
            .iter()
            .zip(self.names.iter())
            .zip(self.rarities.iter())
            .try_for_each(|((a, n), r)| writeln!(f, "{a} {n} ({r})"))?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Deck {
    pub name: String,
    amounts_main: Vec<u32>,
    names_main: Vec<String>,
    amounts_side: Vec<u32>,
    names_side: Vec<String>,
}

impl FromStr for Deck {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parsing_main = true;
        let mut amounts_main = Vec::new();
        let mut names_main = Vec::new();
        let mut amounts_side = Vec::new();
        let mut names_side = Vec::new();
        for (i, l) in s.lines().enumerate() {
            if l.trim().is_empty() {
                parsing_main = false
            }
            let Some((num, name)) = l.split_once(' ') else { continue };
            let num = num.parse().with_context(|| {
                format!("Expected line {} to be of the form `{{integer}} {{card_name}},` but found `{l}`", i+1)
            })?;
            let name = name.to_string();
            if parsing_main {
                amounts_main.push(num);
                names_main.push(name);
            } else {
                amounts_side.push(num);
                names_side.push(name);
            }
        }
        Ok(Deck {
            name: "Unnamed".to_owned(),
            amounts_main,
            amounts_side,
            names_main,
            names_side,
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
}

pub struct Catalogue<P: AsRef<Path>> {
    path: P,
    decks: Vec<Deck>,
}

impl<P: AsRef<Path>> Catalogue<P> {
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
                .map_err(|err| anyhow!("Failed to deserialize catalogue: {err}"))?
        };
        Ok(Catalogue { path, decks })
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

impl<P: AsRef<Path>> Drop for Catalogue<P> {
    fn drop(&mut self) {
        self.write()
            .unwrap_or_else(|err| eprintln!("ERROR: while closing catalogue, {err}"));
    }
}
