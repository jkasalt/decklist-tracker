use indicatif::ProgressBar;
use std::collections::{BTreeSet, HashMap, HashSet};

use crate::{collection::Collection, Rarity, Roster};

struct MissingRares<'a>(BTreeSet<&'a str>);

#[derive(Debug, PartialEq, Eq, Hash, Clone, Default)]
struct IterationState<'a> {
    missing_rares: BTreeSet<&'a str>,
    decks: BTreeSet<usize>,
}

pub struct CraftSuggester<'a, 'b> {
    rares_limit: usize,
    // mythics_limit: usize,
    roster: &'a Roster,
    collection: &'b Collection,
}

impl<'a, 'b> CraftSuggester<'a, 'b> {
    pub fn new(
        rares: usize,
        // mythics: usize,
        roster: &'a Roster,
        collection: &'b Collection,
    ) -> Self {
        CraftSuggester {
            rares_limit: rares,
            // mythics_limit: mythics,
            roster,
            collection,
        }
    }

    /// Returns recommended crafts in the order of importance, following our
    /// homemade cool algorithm.  Given a ``wildcards horizon'', that is the numbere
    /// of wildcards the user is expected to obtain in a certain amount of time
    /// (usually 25 rares for 3 months of play), it maximizes the number of
    /// different number of decks the user can play
    pub fn recommend(&self) {
        let mut mem = HashMap::new();
        let possible = (0..self.roster.len()).collect();
        let mut missing_rares = Vec::new();
        for deck in self.roster.decks() {
            let this_missing_rares = MissingRares(
                self.collection
                    .missing(deck, false)
                    .unwrap()
                    .iter()
                    .filter_map(|(name, _, rarity, _)| {
                        (*rarity == Rarity::Rare).then_some(name.as_str())
                    })
                    .collect(),
            );
            missing_rares.push(this_missing_rares)
        }
        let all_missing_rares: BTreeSet<&&str> =
            missing_rares.iter().flat_map(|mr| &mr.0).collect();
        if all_missing_rares.len() < self.rares_limit {
            println!("You can craft every deck");
        }
        let starting_state = IterationState::default();
        let pb = ProgressBar::new(2u64.pow(self.roster.len() as u32));

        self.recommend_inner(&mut mem, possible, starting_state, &missing_rares, &pb);
        // pb.finish();
        // dbg!(mem);
    }

    fn recommend_inner<'c>(
        &self,
        mem: &mut HashMap<IterationState<'c>, usize>,
        mut possible: Vec<usize>,
        iteration_state: IterationState<'c>,
        missing_rares: &'c [MissingRares],
        pb: &ProgressBar,
    ) -> usize {
        if let Some(val) = mem.get(&iteration_state) {
            return *val;
        }
        pb.inc(1);
        // Remove impossible decks
        possible.retain(|possible_idx| {
            let this_missing_rares = &missing_rares[*possible_idx].0;
            !iteration_state.decks.contains(possible_idx)
                && iteration_state
                    .missing_rares
                    .union(this_missing_rares)
                    .count()
                    <= self.rares_limit
        });
        // Return if we are in an end-case
        if possible.is_empty() {
            let num_decks = iteration_state.decks.len();
            mem.insert(iteration_state, num_decks);
            return num_decks;
        }
        // Otherwise, recurse over all possible decks
        let mut this_iter = HashMap::new(); // keys are "added this index this time" value are best result
        for possible_idx in possible.iter() {
            let mut new_itereation_state = iteration_state.clone();
            new_itereation_state.decks.insert(*possible_idx);
            let this_missing_rares = &missing_rares[*possible_idx].0;
            new_itereation_state
                .missing_rares
                .extend(this_missing_rares);
            let obtained = self.recommend_inner(
                mem,
                possible.clone(),
                new_itereation_state,
                missing_rares,
                pb,
            );
            this_iter.insert(possible_idx, obtained);
        }
        let (_, best_value) = this_iter.into_iter().max_by_key(|(_, v)| *v).unwrap();
        mem.insert(iteration_state, best_value);
        best_value
    }
}
