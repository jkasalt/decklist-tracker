use crate::{
    card_getter::CardGetter, mtga_id_translator::NetCardData, CardData, Deck, Rarity, Roster,
};
use anyhow::{anyhow, Context, Result};
use indicatif::ProgressBar;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    path::Path,
};

pub(crate) fn simplified_name(name: &impl AsRef<str>) -> &str {
    name.as_ref()
        .trim_start_matches("A-")
        .split_once(" // ")
        .map_or(name.as_ref(), |split| split.0)
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Collection {
    content: HashMap<String, Vec<(u8, Rarity, String)>>,
}

impl Collection {
    pub fn from_csv(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file_content =
            fs::read_to_string(path).context("Failed to find collection jsin file")?;

        file_content.lines().enumerate().skip(1).map(|(i, line)| {
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
            Ok((name, amount, rarity, set))
        }).collect::<Result<Collection>>()
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        let collection = serde_json::from_reader(file);
        Ok(collection?)
    }

    fn insert_inner(&mut self, card_data: impl Into<CardData>) {
        let card_data = card_data.into();
        let group = self.content.entry(card_data.name).or_default();
        let to_update = group.iter_mut().find(|cd| cd.2 == card_data.set);
        if let Some(row) = to_update {
            *row = (card_data.amount, card_data.rarity, card_data.set);
        } else {
            group.push((card_data.amount, card_data.rarity, card_data.set));
        }
    }

    pub fn insert(&mut self, card_data: impl Into<CardData>) {
        let card_data = card_data.into();
        let card_name = card_data.name.trim();
        // TODO: do this action _after_ we load everything
        let rarity = if matches!(
            card_name,
            "Plains" | "Island" | "Swamp" | "Mountain" | "Forest"
        ) {
            Rarity::Land
        } else {
            card_data.rarity
        };
        let simplified_name = simplified_name(&card_name).to_owned();
        self.insert_inner((
            card_data.amount,
            simplified_name,
            rarity,
            card_data.set.clone(),
        ));
        self.insert_inner((
            card_data.amount,
            card_name.to_owned(),
            rarity,
            card_data.set,
        ));
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.content.keys()
    }

    pub fn merge(&mut self, other: Collection) {
        for (other_name, other_group) in other.content.into_iter() {
            for other_row in other_group {
                self.insert((other_name.clone(), other_row));
            }
        }
    }

    pub fn ensure_known(&mut self, roster: &Roster) {
        let pb = ProgressBar::new(roster.cards(false).count() as u64);
        for (name, _) in pb.wrap_iter(roster.cards(false)) {
            let name = simplified_name(&name);
            if !self.content.contains_key(name) {
                if let Err(err) = self.fetch_unknown(name) {
                    pb.println(format!("Failed to fetch unknown card: {name}. {err}"));
                }
            }
        }
        pb.finish_and_clear();
    }

    pub fn get(&self, name: impl AsRef<str>) -> Result<&Vec<(u8, Rarity, String)>> {
        let name = simplified_name(&name);
        self.content.get(name).ok_or(anyhow!(
            "Unknown card found: {}. Make sure to run `detr update-collection` before.",
            name,
        ))
    }

    fn fetch_unknown(&mut self, name: impl AsRef<str>) -> Result<()> {
        let card_data = CardGetter::fetch_card(&name)?;
        for NetCardData { name, rarity, set } in card_data {
            self.insert(CardData {
                amount: 0,
                name,
                rarity,
                set,
            });
        }
        Ok(())
    }

    pub fn missing<'a, 'b: 'a>(
        &'a self,
        deck: &'b Deck,
        ignore_sideboard: bool,
    ) -> Result<Vec<(&'b String, u8, Rarity, &'a String)>> {
        let mut missing = Vec::new();
        for (name, deck_amount) in deck.cards(ignore_sideboard) {
            let card_group = self.get(name)?;
            let owned_amount = card_group.iter().map(|(amount, _, _)| amount).sum();
            let (set_name, lowest_rarity) = card_group
                .iter()
                .map(|(_, rarity, set_name)| (set_name, rarity))
                .min_by_key(|(_, rarity)| *rarity)
                .unwrap(); // We can unwrap here because self.get returns early if the card_group is empty
            let missing_amout = deck_amount.saturating_sub(owned_amount);
            missing.push((name, missing_amout, *lowest_rarity, set_name));
        }
        Ok(missing)
    }

    pub fn count_missing_of_rarity(
        &self,
        deck: &Deck,
        ignore_sideboard: bool,
        rarity: Rarity,
    ) -> Result<usize> {
        Ok(self
            .missing(deck, ignore_sideboard)?
            .into_iter()
            .filter_map(|(_, amount, this_rarity, _)| {
                (this_rarity == rarity).then_some(amount as usize)
            })
            .sum())
    }
}

impl FromIterator<(String, u8, Rarity, String)> for Collection {
    fn from_iter<T: IntoIterator<Item = (String, u8, Rarity, String)>>(iter: T) -> Self {
        let mut content = HashMap::new();
        for (name, amount, rarity, set) in iter {
            content
                .entry(name)
                .or_insert_with(Vec::new)
                .push((amount, rarity, set));
        }
        Collection { content }
    }
}
