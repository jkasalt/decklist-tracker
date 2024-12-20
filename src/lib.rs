use crate::collection::Collection;
use anyhow::{anyhow, bail, Context, Result};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::HashMap,
    fs::{self, File},
    io::Write,
    mem,
    path::{Path, PathBuf},
    str::FromStr,
};

pub mod card_getter;
pub mod collection;
pub mod craft_suggester;
pub mod mtga_id_translator;

#[derive(Hash, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Mythic,
    Land,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Eq, PartialEq, Hash, Clone, Serialize, Deserialize)]
pub struct CardData {
    pub amount: u8,
    pub name: String,
    pub rarity: Rarity,
    pub set: String,
}

impl From<(u8, String, Rarity, String)> for CardData {
    fn from(value: (u8, String, Rarity, String)) -> Self {
        Self {
            amount: value.0,
            name: value.1,
            rarity: value.2,
            set: value.3,
        }
    }
}

impl From<(String, (u8, Rarity, String))> for CardData {
    fn from(value: (String, (u8, Rarity, String))) -> Self {
        Self {
            amount: value.1 .0,
            name: value.0,
            rarity: value.1 .1,
            set: value.1 .2,
        }
    }
}

#[derive(Debug)]
pub struct WildcardCoefficients {
    pub common: f32,
    pub uncommon: f32,
    pub rare: f32,
    pub mythic: f32,
}

fn simple_order(a: f32, b: f32) -> Ordering {
    match (b - a).signum() {
        1.0 => Ordering::Less,
        -1.0 => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

impl WildcardCoefficients {
    #[must_use]
    pub fn select(&self, rarity: &Rarity) -> f32 {
        match rarity {
            Rarity::Common => self.common,
            Rarity::Uncommon => self.uncommon,
            Rarity::Rare => self.rare,
            Rarity::Mythic => self.mythic,
            Rarity::Land | Rarity::Unknown => 0.0,
        }
    }

    #[must_use]
    pub fn order(&self) -> [Rarity; 5] {
        use Rarity as R;
        let common = (R::Common, self.common);
        let uncommon = (R::Uncommon, self.uncommon);
        let rare = (R::Rare, self.rare);
        let mythic = (R::Mythic, self.mythic);

        let mut rarities = [common, uncommon, rare, mythic];
        rarities.sort_unstable_by(|(_, c1), (_, c2)| simple_order(*c1, *c2));
        [
            rarities[0].0,
            rarities[1].0,
            rarities[2].0,
            rarities[3].0,
            R::Land,
        ]
    }
}

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct Wildcards {
    pub common: f32,
    pub uncommon: f32,
    pub rare: f32,
    pub mythic: f32,
}

impl Wildcards {
    #[must_use]
    pub fn select(&self, rarity: &Rarity) -> i32 {
        (match rarity {
            Rarity::Common => self.common,
            Rarity::Uncommon => self.uncommon,
            Rarity::Rare => self.rare,
            Rarity::Mythic => self.mythic,
            Rarity::Land | Rarity::Unknown => 0.0,
        })
        .round() as i32
    }

    #[must_use]
    pub fn coefficients(&self) -> WildcardCoefficients {
        let total = self.common + self.uncommon + self.rare + self.mythic;
        let formula = |w| total / (1.0 + w);
        WildcardCoefficients {
            common: formula(self.common),
            uncommon: formula(self.uncommon),
            rare: formula(self.rare),
            mythic: formula(self.mythic),
        }
    }
}

#[derive(Hash, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
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
        for (i, l) in s.lines().skip_while(|l| l.trim().is_empty()).enumerate() {
            match l.trim() {
                "Companion" => {
                    parsing_mode = ParsingMode::Companion;
                    continue;
                }
                "Deck" => {
                    parsing_mode = ParsingMode::Main;
                    continue;
                }
                "Sideboard" | "" => {
                    parsing_mode = ParsingMode::Side;
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
                .and_then(|w| w.parse().ok())
                .with_context(error_message)?;
            let name = words.take_while(|w| !w.starts_with('(')).join(" ");
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
        Ok(Self {
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
    #[must_use]
    pub fn name(self, name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..self
        }
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        fs::read_to_string(path)?.parse()
    }

    pub fn cards(&self, ignore_sideboard: bool) -> impl Iterator<Item = (&String, u8)> {
        let mut cards_amounts = HashMap::new();
        let mainboard_iterator = self.amounts_main.iter().zip(self.names_main.iter());
        let has_wishboard = self
            .names_main
            .contains(&"Karn, the Great Creator".to_owned());
        for (amount, card_name) in mainboard_iterator {
            *cards_amounts.entry(card_name).or_insert(0) += amount;
        }
        if !ignore_sideboard {
            let sideboard_iterator = self.amounts_side.iter().zip(self.names_side.iter());
            for (amount, card_name) in sideboard_iterator {
                *cards_amounts.entry(card_name).or_insert(0) += amount;
            }
        }
        if ignore_sideboard && has_wishboard {
            let sideboard_iterator = self.amounts_side.iter().zip(self.names_side.iter());
            for (amount, card_name) in sideboard_iterator.take(7) {
                *cards_amounts.entry(card_name).or_insert(0) += amount;
            }
        }

        cards_amounts.into_iter()
    }

    pub fn contains(&self, s: &impl PartialEq<String>, ignore_sideboard: bool) -> bool {
        (!ignore_sideboard && self.names_side.iter().any(|ns| s.eq(ns)))
            || self.names_main.iter().any(|nm| s.eq(nm))
    }
}

#[derive(Debug)]
pub struct Roster {
    path: PathBuf,
    decks: Vec<Deck>,
}

impl Roster {
    pub fn decks_mut(&mut self) -> std::slice::IterMut<Deck> {
        self.decks.iter_mut()
    }
    pub fn decks(&self) -> std::slice::Iter<Deck> {
        self.decks.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.decks.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn get(&self, n: usize) -> Option<&Deck> {
        self.decks.get(n)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = if path.as_ref().exists() {
            File::open(&path)?
        } else {
            let mut file = File::create(&path)?;
            file.write_all(b"[]")?;
            file
        };
        let decks = serde_json::from_reader(file)
            .map_err(|err| anyhow!("Failed to deserialize roster: {err}"))?;
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            decks,
        })
    }

    // TODO: change &Deck to Generic Cow<Deck>
    pub fn add_deck(&mut self, deck: Deck) {
        self.decks.push(deck);
    }

    pub fn remove_deck(&mut self, name: &str) -> Result<()> {
        let i = self
            .decks()
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

    pub fn deck_names(&self) -> impl Iterator<Item = &str> {
        self.decks().map(|deck| deck.name.as_str())
    }

    pub fn find(&self, deck_name: &str) -> Result<&Deck> {
        self.decks()
            .find(|deck| deck.name == deck_name)
            .ok_or(anyhow!("Could not find deck {deck_name} in roster"))
    }

    pub fn find_mut(&mut self, deck_name: &str) -> Result<&mut Deck> {
        self.decks_mut()
            .find(|deck| deck.name == deck_name)
            .ok_or(anyhow!("Could not find deck {deck_name} in roster"))
    }

    pub fn replace(&mut self, deck_name: &str, deck: Deck) -> Result<()> {
        let in_roster = self.find_mut(deck_name)?;
        *in_roster = deck;
        Ok(())
    }

    pub fn cards(&self, ignore_sideboard: bool) -> impl Iterator<Item = (&String, u8)> {
        self.decks
            .iter()
            .flat_map(move |deck| deck.cards(ignore_sideboard))
    }
}

impl Drop for Roster {
    fn drop(&mut self) {
        self.write()
            .unwrap_or_else(|err| eprintln!("ERROR: while closing roster, {err}"));
    }
}

#[derive(Debug)]
pub struct Inventory {
    collection: Collection,
    collection_path: PathBuf,
    coeffs: WildcardCoefficients,
}

impl Inventory {
    pub fn open<P1, P2>(collection_path: P1, wildcards_path: P2) -> Result<Self>
    where
        P1: AsRef<Path> + std::fmt::Debug,
        P2: AsRef<Path> + std::fmt::Debug,
    {
        if !collection_path.as_ref().exists() {
            fs::write(
                &collection_path,
                serde_json::to_string(&Collection::default())?,
            )?;
        }
        let collection: Collection = serde_json::from_reader(File::open(&collection_path)?)
            .with_context(|| format!("Failed to open collection with path {collection_path:?}"))?;
        let wildcards: Wildcards = if wildcards_path.as_ref().exists() {
            serde_json::from_reader(File::open(&wildcards_path)?).unwrap_or_default()
        } else {
            Wildcards::default()
        };
        let coeffs = wildcards.coefficients();
        Ok(Self {
            collection,
            coeffs,
            collection_path: collection_path.as_ref().to_path_buf(),
        })
    }

    pub fn card_cost(&self, card_name: &str) -> Result<f32> {
        let cheapest_rarity = &self.cheapest_rarity(card_name)?;
        let cost = self.coeffs.select(cheapest_rarity);
        Ok(cost)
    }

    /// This function computes the importance of a card, with regard to how many
    /// copies a deck plays.
    pub fn card_cost_considering_deck(
        &mut self,
        card_name: &str,
        in_deck_amount: u8,
    ) -> Result<f32> {
        let in_collection_amount = self.card_amount(card_name)?;
        let missing = in_deck_amount.saturating_sub(in_collection_amount);
        if missing == 0 {
            Ok(0.0)
        } else {
            let tiebreaker_bonus = 4.0 / f32::from(missing);
            Ok((self.card_cost(card_name)?).mul_add(f32::from(missing), tiebreaker_bonus))
        }
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn cheapest_rarity(&self, card_name: &str) -> Result<Rarity> {
        let card_group = self.collection.get(card_name)?;
        let group_rarities = card_group.iter().map(|(_, rarity, _)| rarity).collect_vec();
        let ordered_rarities = self.coeffs.order();
        let cheapest_rarity = ordered_rarities
            .iter()
            .find(|r| group_rarities.contains(r))
            .unwrap();
        Ok(*cheapest_rarity)
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn cheapest_version<'a>(&'a self, card_name: &'a str) -> Result<&(u8, Rarity, String)> {
        let cheapest_rarity = self.cheapest_rarity(card_name)?;
        let card_group = self.collection.get(card_name)?;
        let cheapest_version = card_group
            .iter()
            .find(|(_, rarity, _)| *rarity == cheapest_rarity)
            .unwrap();
        Ok(cheapest_version)
    }

    pub fn card_amount(&self, card_name: &str) -> Result<u8> {
        let in_collection: u8 = self
            .collection
            .get(card_name)?
            .iter()
            .map(|(amount, _, _)| *amount)
            .sum();
        Ok(in_collection.clamp(0, 4))
    }

    pub fn deck_cost(&mut self, deck: &Deck, ignore_sideboard: bool) -> Result<f32> {
        let mut result = 0.0;
        for (card_name, amount) in deck.cards(ignore_sideboard) {
            let missing = amount.saturating_sub(self.card_amount(card_name)?);
            result += f32::from(missing) * self.card_cost(card_name)?;
        }
        if result.abs() < f32::EPSILON {
            return Ok(0.0);
        }
        let closeness_bound = self.rare_coeff().mul_add(4.0, self.mythic_coeff());
        let cool_formula = f32::max(result - closeness_bound, 1.00);
        Ok(cool_formula)
    }

    pub fn update_collection(&mut self, recently_fetched: Collection, roster: &Roster) {
        self.collection.ensure_known(roster);
        let mut original = mem::take(&mut self.collection);
        original.merge(recently_fetched);
        mem::swap(&mut original, &mut self.collection);
    }

    pub fn get<'b>(&'b mut self, s: &'b str) -> Result<&Vec<(u8, Rarity, String)>> {
        self.collection.get(s)
    }

    pub fn missing_cards<'b>(
        &'b self,
        deck: &'b Deck,
        ignore_sideboard: bool,
    ) -> Result<Vec<(&String, u8, Rarity, &String)>> {
        self.collection.missing(deck, ignore_sideboard)
    }

    #[must_use]
    pub fn wildcard_coeffs(&self) -> &WildcardCoefficients {
        &self.coeffs
    }

    #[must_use]
    pub fn common_coeff(&self) -> f32 {
        self.coeffs.common
    }

    #[must_use]
    pub fn uncommon_coeff(&self) -> f32 {
        self.coeffs.uncommon
    }

    #[must_use]
    pub fn rare_coeff(&self) -> f32 {
        self.coeffs.rare
    }

    #[must_use]
    pub fn mythic_coeff(&self) -> f32 {
        self.coeffs.mythic
    }
}

impl Drop for Inventory {
    fn drop(&mut self) {
        fs::write(
            &self.collection_path,
            serde_json::to_string(&self.collection).unwrap(),
        )
        .unwrap_or_else(|err| eprintln!("ERROR: When closing inventory {err}"));
    }
}
