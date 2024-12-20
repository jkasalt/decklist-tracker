use bitvec::{bitvec, vec::BitVec};
use itertools::Itertools;
use std::collections::{BTreeSet, HashMap};

use crate::{collection::Collection, Deck, Rarity, Roster};

fn build_matrix(decks: &[&Deck], rows_index: &BTreeSet<(&String, u8)>) -> Vec<BitVec> {
    let mut columns: Vec<BitVec> = Vec::new();
    for deck in decks {
        let mut column = vec![false; rows_index.len()];
        for (i, (rare, _)) in rows_index.iter().enumerate() {
            column[i] = deck.contains(*rare, false);
        }
        let column = BitVec::from_iter(column);
        columns.push(column);
    }
    columns
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Default)]
struct IterationState<'a> {
    missing_rares: BTreeSet<&'a str>,
    decks: BTreeSet<usize>,
}

pub struct CraftRecommender<'r, 'c> {
    rares_limit: usize,
    mythics_limit: usize,
    roster: &'r Roster,
    collection: &'c Collection,
    starting_sel: Vec<String>,
    ignore_sb: bool,
}

impl<'r, 'c> CraftRecommender<'r, 'c> {
    #[must_use]
    pub fn new(
        rares: usize,
        mythics: usize,
        ignore_sideboard: bool,
        starting_sel: Option<Vec<String>>,
        roster: &'r Roster,
        collection: &'c Collection,
    ) -> Self {
        CraftRecommender {
            rares_limit: rares,
            mythics_limit: mythics,
            ignore_sb: ignore_sideboard,
            starting_sel: starting_sel.unwrap_or_default(),
            roster,
            collection,
        }
    }

    fn build_rows_index(&self, decks: &[&'r Deck], target: Rarity) -> BTreeSet<(&String, u8)> {
        let mut rows_index = BTreeSet::new();
        for deck in decks {
            let missing = self.collection.missing(deck, self.ignore_sb).unwrap();
            for (name, amount, rarity, _) in missing {
                if rarity == target {
                    for n in 1..=amount {
                        rows_index.insert((name, n));
                    }
                }
            }
        }
        rows_index
    }

    fn relevant_decks(&self) -> Vec<&Deck> {
        self.roster
            .decks()
            .filter(|deck| {
                let missing_rares = self
                    .collection
                    .count_missing_of_rarity(deck, self.ignore_sb, Rarity::Rare)
                    .unwrap();
                let missing_mythics = self
                    .collection
                    .count_missing_of_rarity(deck, self.ignore_sb, Rarity::Mythic)
                    .unwrap();
                0 < missing_rares + missing_mythics
                    && missing_rares <= self.rares_limit
                    && missing_mythics <= self.mythics_limit
            })
            .collect()
    }

    /// Returns recommended crafts in the order of importance, following our
    /// homemade cool algorithm.  Given a `wildcards horizon`, that is the number
    /// of wildcards the user is expected to obtain in a certain amount of time
    /// (usually 25 rares for 3 months of play), it maximizes the number of
    /// different number of decks the user can play
    #[must_use]
    pub fn recommend(&self) -> Vec<Vec<&str>> {
        // Get the decks for which we are missing at least a rare or mythic card
        let decks: Vec<_> = self.relevant_decks();

        // Build the memo table
        let mut mem = HashMap::new();

        // Build the matrix
        // Get the rows
        let all_rares = self.build_rows_index(&decks, Rarity::Rare);
        let all_mythics = self.build_rows_index(&decks, Rarity::Mythic);

        // Get the columns
        let columns_rares = build_matrix(&decks, &all_rares);
        let columns_mythics = build_matrix(&decks, &all_mythics);

        // Get the starting sel
        let starting_bits = decks
            .iter()
            .map(|deck| self.starting_sel.contains(&deck.name))
            .collect();

        // Iterate over all possible combinations
        self.recommend_inner(&mut mem, &columns_rares, &columns_mythics, starting_bits);

        // Collect the result
        let result: Vec<_> = mem.into_iter().max_set_by(|(k1, v1), (k2, v2)| {
            let d1 = k1.iter_ones().count();
            let d2 = k2.iter_ones().count();
            v1.cmp(v2).then(d1.cmp(&d2))
        });

        // Turn the codes back into deck names
        let decks_codes: Vec<_> = result.iter().map(|(decks, _)| decks).collect();
        decks_codes
            .iter()
            .map(|decks_code| {
                decks
                    .iter()
                    .enumerate()
                    .filter_map(|(i, deck)| (decks_code[i]).then_some(deck.name.as_str()))
                    .collect()
            })
            .collect()
    }

    fn recommend_inner(
        &self,
        mem: &mut HashMap<BitVec, usize>,
        columns_rares: &[BitVec],
        columns_mythics: &[BitVec],
        current_sel: BitVec,
    ) -> usize {
        if let Some(val) = mem.get(&current_sel) {
            return *val;
        }
        // Get possible next steps
        let possible: Vec<_> = (0..columns_rares.len())
            .map(|j| current_sel.clone() | BitVec::from_element(1 << j))
            .filter(|new_sel| *new_sel != current_sel)
            .filter(|new_sel| {
                let count_nonzero = |columns: &[BitVec]| {
                    columns
                        .iter()
                        .enumerate()
                        .filter_map(|(i, c)| (new_sel[i]).then_some(c)) // Pick out the columns that belong to new_sel
                        .fold(bitvec!(0; columns[0].len()), |acc, x| acc | x) // Join them
                        .iter_ones()
                        .count()
                };
                let num_selected_rares = count_nonzero(columns_rares);
                let num_selected_mythics = count_nonzero(columns_mythics);
                num_selected_rares <= self.rares_limit && num_selected_mythics <= self.mythics_limit
            })
            .collect();
        if possible.is_empty() {
            let result = current_sel.iter_ones().count();
            mem.insert(current_sel, result);
            return result;
        }
        let best = possible
            .iter()
            .map(|new_sel| {
                self.recommend_inner(mem, columns_rares, columns_mythics, new_sel.clone())
            })
            .max()
            .unwrap();
        mem.insert(current_sel, best);
        best
    }
}
