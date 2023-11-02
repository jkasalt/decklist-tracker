use crate::{
    card_getter::{self, CardGetter},
    CardData, Deck, Rarity, Roster,
};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

fn simplified_name(name: &impl AsRef<str>) -> &str {
    name.as_ref()
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
            fs::read_to_string(path).context("Failed to find collection csv file")?;

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
        let simplified_name = simplified_name(&card_name).to_owned();
        self.insert_inner((
            card_data.amount,
            simplified_name,
            card_data.rarity,
            card_data.set.clone(),
        ));
        self.insert_inner((
            card_data.amount,
            card_name.to_owned(),
            card_data.rarity,
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
        for (name, _) in roster.cards(false) {
            if !self.content.contains_key(name) {
                if let Err(err) = self.fetch_unknown(name) {
                    eprintln!("Failed to fetch unknown card: {name}. {err}");
                }
            }
        }
    }

    pub fn get(&self, name: impl AsRef<str>) -> Result<&Vec<(u8, Rarity, String)>> {
        self.content.get(name.as_ref()).ok_or(anyhow!(
            "Unknown card found: {}. Make sure to run `detr update-collection` before.",
            name.as_ref()
        ))
    }

    fn fetch_unknown(&mut self, name: impl AsRef<str>) -> Result<()> {
        eprintln!("Fetching unknown card: {}", name.as_ref());
        let card_data = CardGetter::fetch_card(&name)?;
        self.insert(CardData {
            amount: 0,
            name: card_data.name,
            rarity: card_data.rarity,
            set: card_data.set,
        });
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
