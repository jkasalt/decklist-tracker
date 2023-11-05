use crate::collection::Collection;
use anyhow::{anyhow, bail, Context, Result};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    mem,
    path::{Path, PathBuf},
    str::FromStr,
};

pub mod card_getter;
pub mod collection;
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
        CardData {
            amount: value.0,
            name: value.1,
            rarity: value.2,
            set: value.3,
        }
    }
}

impl From<(String, (u8, Rarity, String))> for CardData {
    fn from(value: (String, (u8, Rarity, String))) -> Self {
        CardData {
            amount: value.1 .0,
            name: value.0,
            rarity: value.1 .1,
            set: value.1 .2,
        }
    }
}

// impl CardData {
//     pub fn as_ref(&self) -> RefCardData {
//         RefCardData {
//             amount: &self.amount,
//             name: &self.name,
//             rarity: &self.rarity,
//             set_name: &self.set_name,
//         }
//     }
// }

// #[derive(Debug, Clone)]
// pub struct RefCardData<'a> {
//     pub amount: &'a u8,
//     pub name: &'a str,
//     pub rarity: &'a Rarity,
//     pub set_name: &'a str,
// }

// impl<'a> RefCardData<'a> {
//     pub fn to_owned(&self) -> CardData {
//         CardData {
//             amount: *self.amount,
//             name: self.name.to_owned(),
//             rarity: *self.rarity,
//             set_name: self.set_name.to_owned(),
//         }
//     }
//     pub fn simplified_name(&self) -> &str {
//         self.name
//             .split_once(" // ")
//             .map_or(self.name, |split| split.0)
//     }
// }

#[derive(Debug)]
pub struct WildcardCoefficients {
    pub common: f32,
    pub uncommon: f32,
    pub rare: f32,
    pub mythic: f32,
}

impl WildcardCoefficients {
    pub fn select(&self, rarity: &Rarity) -> f32 {
        match rarity {
            Rarity::Common => self.common,
            Rarity::Uncommon => self.uncommon,
            Rarity::Rare => self.rare,
            Rarity::Mythic => self.mythic,
            Rarity::Land | Rarity::Unknown => 0.0,
        }
    }

    pub fn order(&self) -> [Rarity; 5] {
        use Rarity as R;
        let common = (R::Common, self.common);
        let uncommon = (R::Uncommon, self.uncommon);
        let rare = (R::Rare, self.rare);
        let mythic = (R::Mythic, self.mythic);

        let mut rarities = [common, uncommon, rare, mythic];
        rarities.sort_unstable_by(|(_, c1), (_, c2)| c1.partial_cmp(c2).unwrap());
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
    pub common: u32,
    pub uncommon: u32,
    pub rare: u32,
    pub mythic: u32,
}

impl Wildcards {
    pub fn select(&self, rarity: &Rarity) -> u32 {
        match rarity {
            Rarity::Common => self.common,
            Rarity::Uncommon => self.uncommon,
            Rarity::Rare => self.rare,
            Rarity::Mythic => self.mythic,
            Rarity::Land | Rarity::Unknown => 0,
        }
    }

    pub fn coefficients(&self) -> WildcardCoefficients {
        let total = self.common + self.uncommon + self.rare + self.mythic;
        WildcardCoefficients {
            common: total as f32 / (1 + self.common) as f32,
            uncommon: total as f32 / (1 + self.uncommon) as f32,
            rare: total as f32 / (1 + self.rare) as f32,
            mythic: total as f32 / (1 + self.mythic) as f32,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
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
        Ok(Roster {
            path: path.as_ref().to_path_buf(),
            decks,
        })
    }

    // TODO: change &Deck to Generic Cow<Deck>
    pub fn add_deck(&mut self, deck: &Deck) {
        self.decks.push(deck.clone());
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

    pub fn deck_list(&self) -> impl Iterator<Item = &str> {
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
        let wildcards: Wildcards = if !wildcards_path.as_ref().exists() {
            Wildcards::default()
        } else {
            serde_json::from_reader(File::open(&wildcards_path)?).unwrap_or_default()
        };
        let coeffs = wildcards.coefficients();
        Ok(Inventory {
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
    /// copies a deck plays. A card that is played as a four-of will always be
    /// more important than a card that is played as a single. This is a
    /// helpful heuristic most of the time. However, care must be taken when
    /// considering some decks that play important single cards, such as
    /// Approach of the second sun, or Atraxa reanimator decks.
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
            let tiebreaker_bonus = 4.0 / missing as f32;
            Ok(self.card_cost(card_name)? * in_deck_amount as f32 + tiebreaker_bonus)
        }
    }

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
            result += missing as f32 * self.card_cost(card_name)?;
        }
        let closeness_bound = self.rare_coeff() * 4.0 + self.mythic_coeff();
        let nifty_formula = f32::max(result - closeness_bound, 1.00);
        Ok(nifty_formula)
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

    pub fn wildcard_coeffs(&self) -> &WildcardCoefficients {
        &self.coeffs
    }

    pub fn common_coeff(&self) -> f32 {
        self.coeffs.common
    }

    pub fn uncommon_coeff(&self) -> f32 {
        self.coeffs.uncommon
    }

    pub fn rare_coeff(&self) -> f32 {
        self.coeffs.rare
    }

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
