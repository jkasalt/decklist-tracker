use anyhow::{anyhow, bail, Context};
use clap::{arg, Parser, Subcommand};
use decklist_tracker::{CardData, Collection, Deck, Rarity, Roster};
use directories::BaseDirs;
use std::{
    collections::HashMap,
    fs::{self},
    path::PathBuf,
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(subcommand_required = true)]
struct Cli {
    #[arg(short, long, global = true)]
    roster_path: Option<String>,

    #[arg(short, long, global = true)]
    collection_path: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    AddFromFile {
        deck_paths: Vec<String>,

        #[arg(long, short)]
        names: Option<Vec<String>>,
    },
    Paste {
        name: String,
    },
    Missing {
        deck_name: String,
    },
    UpdateCollection {
        path: String,
    },
    Remove {
        deck_name: String,
    },
    ShowDeck {
        deck_name: String,
    },
    Suggest,
    List,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let base_dirs = BaseDirs::new().ok_or(anyhow!("Could not obtain system's base directories"))?;
    let app_dir = base_dirs.data_dir().join("my_projects/decklist_tracker");
    if !app_dir.exists() {
        fs::create_dir_all(&app_dir).context("Failed to create directory for app data")?;
    }
    let roster_path = cli
        .roster_path
        .map(PathBuf::from)
        .unwrap_or_else(|| app_dir.join("roster.json"));
    let collection_path = cli
        .collection_path
        .map(PathBuf::from)
        .unwrap_or_else(|| app_dir.join("collection.csv"));
    let mut roster = Roster::open(&roster_path)
        .with_context(|| format!("Failed to open deck roster with path {roster_path:?}"))?;
    let collection = Collection::from_csv(&collection_path)
        .with_context(|| format!("Failed to open collection with path {collection_path:?}"))?;
    match cli.command {
        Some(Commands::ShowDeck { deck_name }) => {
            if let Some(deck) = roster.iter().find(|in_roster| deck_name == in_roster.name) {
                println!("{deck}");
            } else {
                bail!("Could not find {deck_name} in roster {roster_path:?}");
            }
        }
        Some(Commands::Remove { deck_name }) => {
            roster
                .remove_deck(&deck_name)
                .context("Failed to remove deck")?;
        }
        Some(Commands::AddFromFile { deck_paths, names }) => {
            let names_iter = if let Some(names) = names {
                names.into_iter()
            } else {
                vec!["Unnamed".to_owned(); deck_paths.len()].into_iter()
            };
            let decks: Vec<Deck> = deck_paths
                .iter()
                .zip(names_iter)
                .map(|(deck_path, name)| {
                    let deck = fs::read_to_string(deck_path)
                        .context("Failed to find decklist")?
                        .parse::<Deck>()
                        .context("Failed to parse decklist")?
                        .name(&name);
                    Ok(deck)
                })
                .collect::<anyhow::Result<Vec<Deck>>>()?;
            for deck in decks {
                roster.add_deck(&deck);
            }
        }
        Some(Commands::Paste { name }) => {
            let deck: Deck =
                clipboard_win::get_clipboard::<String, _>(clipboard_win::formats::Unicode)
                    .map_err(|err| anyhow!("Failed to read clipboard: {err}"))?
                    .parse::<Deck>()
                    .context("Failed to parse deck from clipboard")?
                    .name(&name);
            roster.add_deck(&deck);
        }
        Some(Commands::List) => {
            roster.deck_list().for_each(|name| println!("{name}"));
        }
        Some(Commands::Missing { deck_name }) => {
            match roster
                .iter()
                .find(|in_cat| in_cat.name == deck_name)
                .map(|deck| collection.missing(deck))
            {
                Some(missing_cards) => {
                    let mut missing_cards = missing_cards?;
                    missing_cards.sort_by_key(|m| m.rarity);
                    let missing_rares: u8 = missing_cards
                        .iter()
                        .map(|m| {
                            if m.rarity == Rarity::Rare {
                                m.amount
                            } else {
                                0
                            }
                        })
                        .sum();
                    let missing_mythics: u8 = missing_cards
                        .iter()
                        .map(|m| {
                            if m.rarity == Rarity::Mythic {
                                m.amount
                            } else {
                                0
                            }
                        })
                        .sum();
                    println!(
                        "Missing rares: {missing_rares}. Missing mythics: {missing_mythics}.\n"
                    );
                    missing_cards
                        .iter()
                        .filter(|m| m.amount > 0)
                        .for_each(|missing| {
                            println!("{:?}\t {} {}", missing.rarity, missing.amount, missing.name);
                        })
                }
                None => anyhow::bail!("Cannot find deck {deck_name} in deck roster"),
            }
        }
        Some(Commands::UpdateCollection { path }) => {
            std::fs::copy(path, collection_path)?;
        }
        Some(Commands::Suggest) => {
            let collection = collection.into_hash_map();
            let mut sug_common = HashMap::new();
            let mut sug_uncommon = HashMap::new();
            let mut sug_rare = HashMap::new();
            let mut sug_mythic = HashMap::new();
            for deck in roster.iter() {
                for (amount_deck, card_name) in deck.cards() {
                    if let "Plains" | "Island" | "Swamp" | "Mountain" | "Forest" =
                        card_name.as_str()
                    {
                        continue;
                    }
                    let (amount_coll, rarity) =
                        collection.get(card_name).unwrap_or(&(0, Rarity::Rare));
                    let needed = amount_deck.saturating_sub(*amount_coll);
                    if needed == 0 {
                        continue;
                    }
                    match rarity {
                        Rarity::Common => *sug_common.entry(card_name).or_insert(0) += needed,
                        Rarity::Uncommon => *sug_uncommon.entry(card_name).or_insert(0) += needed,
                        Rarity::Rare => *sug_rare.entry(card_name).or_insert(0) += needed,
                        Rarity::Mythic => *sug_mythic.entry(card_name).or_insert(0) += needed,
                        Rarity::Land => {}
                    }
                }
            }

            let suggestions = vec![
                (sug_common, "Common"),
                (sug_uncommon, "Uncommon"),
                (sug_rare, "Rare"),
                (sug_mythic, "Mythic Rare"),
            ];
            for suggestion_group in suggestions {
                println!("{}", suggestion_group.1);
                let mut suggestion_group: Vec<_> = suggestion_group.0.into_iter().collect();
                suggestion_group.sort_unstable_by_key(|(_, amount)| std::cmp::Reverse(*amount));
                for (name, amount) in suggestion_group.iter().take(15) {
                    println!("{amount} {name}");
                }
                println!();
            }
        }
        None => {}
    }
    Ok(())
}
