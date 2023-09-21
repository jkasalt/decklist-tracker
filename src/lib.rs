use anyhow::{anyhow, bail, Context, Result};
use either::*;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    str::FromStr,
};

#[derive(Hash, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Mythic,
    Land,
    Unknown,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct CardData {
    pub amount: u8,
    pub name: String,
    pub rarity: Rarity,
    pub set_name: String,
}

impl CardData {
    pub fn as_ref(&self) -> RefCardData {
        RefCardData {
            amount: &self.amount,
            name: &self.name,
            rarity: &self.rarity,
            set_name: &self.set_name,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RefCardData<'a> {
    pub amount: &'a u8,
    pub name: &'a str,
    pub rarity: &'a Rarity,
    pub set_name: &'a str,
}

impl<'a> RefCardData<'a> {
    pub fn to_owned(&self) -> CardData {
        CardData {
            amount: *self.amount,
            name: self.name.to_owned(),
            rarity: *self.rarity,
            set_name: self.set_name.to_owned(),
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

#[derive(Debug)]
pub struct Collection {
    names: Vec<String>,
    amounts: Vec<u8>,
    rarities: Vec<Rarity>,
    sets: Vec<String>,
}

impl Collection {
    pub fn from_csv(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).context("Failed to find collection csv file")?;
        let num_lines = content.lines().count();
        let mut names = Vec::with_capacity(num_lines);
        let mut amounts = Vec::with_capacity(num_lines);
        let mut rarities = Vec::with_capacity(num_lines);
        let mut sets = Vec::with_capacity(num_lines);

        for (i, line) in content.lines().enumerate().skip(1) {
            let err_message = || {
                format!("Failed to read line {line} (number {i}) in file {path:?}, as it is not in the expected format")
            };
            let mut elements = line.split(';');
            let amount = elements.next().with_context(err_message)?.parse()?;
            let name = elements.next().with_context(err_message)?.to_owned();
            let set = elements.next().with_context(err_message)?.to_owned();
            let rarity = elements
                .nth(1)
                .map(|s| match s {
                    "common" => Rarity::Common,
                    "uncommon" => Rarity::Uncommon,
                    "rare" => Rarity::Rare,
                    "mythic" => Rarity::Mythic,
                    "land" => Rarity::Land,
                    _ => Rarity::Unknown,
                })
                .with_context(err_message)?;
            names.push(name);
            amounts.push(amount);
            sets.push(set);
            rarities.push(rarity);
        }

        Ok(Collection {
            names,
            amounts,
            rarities,
            sets,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = RefCardData> {
        self.amounts
            .iter()
            .zip(self.names.iter())
            .zip(self.rarities.iter())
            .zip(self.sets.iter())
            .map(|(((amount, name), rarity), set_name)| RefCardData {
                amount,
                name,
                rarity,
                set_name,
            })
    }

    pub fn get<'a>(&'a self, s: &'a str) -> Option<Vec<RefCardData>> {
        if let "Plains" | "Island" | "Swamp" | "Mountain" | "Forest" = s {
            return Some(vec![RefCardData {
                amount: &u8::MAX,
                name: s,
                rarity: &Rarity::Land,
                set_name: "land",
            }]);
        }
        let found = self
            .iter()
            .filter(|card_data| card_data.name == s)
            .collect_vec();
        if found.is_empty() {
            None
        } else {
            Some(found)
        }
    }

    pub fn missing<'a>(
        &'a self,
        deck: &'a Deck,
        ignore_sideboard: bool,
    ) -> impl Iterator<Item = CardData> + 'a {
        deck.cards(ignore_sideboard)
            .filter(|(_, deck_card_name)| {
                // Ignore basic lands
                !matches!(
                    deck_card_name.as_str(),
                    "Plains" | "Island" | "Swamp" | "Mountain" | "Forest"
                )
            })
            // For each card in the deck
            .map(|(deck_amount, name)| {
                let card_group: Vec<_> = self
                    .iter()
                    .filter(|refcard_data| refcard_data.name == name)
                    .collect();
                let owned_amount = card_group.iter().map(|card_data| card_data.amount).sum();
                let (set_name, lowest_rarity) = card_group
                    .iter()
                    .map(|card_data| (card_data.set_name, card_data.rarity))
                    .min_by_key(|(_, rarity)| *rarity)
                    .unwrap_or(("???", &Rarity::Unknown));
                let missing_amout = deck_amount.saturating_sub(owned_amount);
                CardData {
                    amount: missing_amout,
                    name: name.to_owned(),
                    rarity: *lowest_rarity,
                    set_name: set_name.to_string(),
                }
            })
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

    pub fn cards(&self, ignore_sideboard: bool) -> impl Iterator<Item = (&u8, &String)> {
        let cards_iterator = self.amounts_main.iter().zip(self.names_main.iter());
        if ignore_sideboard {
            Left(cards_iterator)
        } else {
            Right(cards_iterator.chain(self.amounts_side.iter().zip(self.names_side.iter())))
        }
    }
}

#[derive(Debug)]
pub struct Roster<P: AsRef<Path>> {
    path: P,
    decks: Vec<Deck>,
}

impl<P: AsRef<Path>> Roster<P> {
    pub fn decks_mut(&mut self) -> std::slice::IterMut<Deck> {
        self.decks.iter_mut()
    }
    pub fn decks(&self) -> std::slice::Iter<Deck> {
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
}

impl<P: AsRef<Path>> Drop for Roster<P> {
    fn drop(&mut self) {
        self.write()
            .unwrap_or_else(|err| eprintln!("ERROR: while closing roster, {err}"));
    }
}

#[derive(Debug)]
pub struct Inventory {
    collection: Collection,
    coeffs: WildcardCoefficients,
}

impl Inventory {
    pub fn open<P1, P2>(collection_path: P1, wildcards_path: P2) -> Result<Self>
    where
        P1: AsRef<Path> + std::fmt::Debug,
        P2: AsRef<Path> + std::fmt::Debug,
    {
        let collection = Collection::from_csv(&collection_path)
            .with_context(|| format!("Failed to open collection with path {collection_path:?}"))?;
        let wildcards: Wildcards = if !wildcards_path.as_ref().exists() {
            Wildcards::default()
        } else {
            serde_json::from_reader(File::open(&wildcards_path)?).unwrap_or_default()
        };
        let coeffs = wildcards.coefficients();
        Ok(Inventory { collection, coeffs })
    }

    pub fn card_cost(&self, card_name: &str) -> Result<f32> {
        let cost = self.coeffs.select(&self.cheapest_rarity(card_name)?);
        Ok(cost)
    }

    /// This function computes the importance of a card, with regard to how many
    /// copies a deck plays. A card that is played as a four-of will always be
    /// more important than a card that is played as a single. This is a
    /// helpful heuristic most of the time. However, care must be taken when
    /// considering some decks that play important single cards, such as
    /// Approach of the second sun, or Atraxa reanimator decks.
    pub fn card_cost_considering_deck(&self, card_name: &str, &in_deck_amount: &u8) -> Result<f32> {
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
        let card_group = self
            .collection
            .get(card_name)
            .ok_or(anyhow!("Failed to find card {card_name} in collection"))?;
        let group_rarities = card_group
            .iter()
            .map(|card_data| card_data.rarity)
            .collect_vec();
        let ordered_rarities = self.coeffs.order();
        let cheapest_rarity = ordered_rarities
            .iter()
            .find(|r| group_rarities.contains(r))
            .unwrap();
        Ok(*cheapest_rarity)
    }

    pub fn cheapest_version(&self, card_name: &str) -> Result<CardData> {
        let cheapest_rarity = self.cheapest_rarity(card_name)?;
        let card_group = self
            .collection
            .get(card_name)
            .ok_or(anyhow!("Cannot find {card_name} in collection"))?;
        let cheapest_version = card_group
            .iter()
            .find(|card_data| *card_data.rarity == cheapest_rarity)
            .unwrap();
        Ok(cheapest_version.to_owned())
    }

    pub fn card_amount(&self, card_name: &str) -> Result<u8> {
        let in_collection = self
            .collection
            .get(card_name)
            .ok_or(anyhow!("Failed to find card {card_name} in collection"))?
            .iter()
            .map(|card_data| *card_data.amount)
            .sum();
        Ok(std::cmp::min(in_collection, 4))
    }

    pub fn deck_cost(&self, deck: &Deck, ignore_sideboard: bool) -> Result<f32> {
        let mut result = 0.0;
        for (amount, card_name) in deck.cards(ignore_sideboard) {
            let missing = amount.saturating_sub(self.card_amount(card_name)?);
            result += missing as f32 * self.card_cost(card_name)?;
        }
        Ok(result)
    }

    pub fn missing_cards<'a>(
        &'a self,
        deck: &'a Deck,
        ignore_sideboard: bool,
    ) -> impl Iterator<Item = CardData> + 'a {
        self.collection.missing(deck, ignore_sideboard)
    }

    pub fn wildcard_coeffs(&self) -> &WildcardCoefficients {
        &self.coeffs
    }
}
