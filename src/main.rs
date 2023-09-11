use anyhow::{anyhow, Context, Result};
use clap::{arg, Parser, Subcommand};
use detr::{CardData, Deck, Inventory, Rarity, Roster, Wildcards};
use directories::BaseDirs;
use either::*;
use itertools::Itertools;
use regex::Regex;
use std::{
    collections::HashMap,
    fs::{self},
    path::{Path, PathBuf},
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
    Show {
        deck_name: String,
    },
    Export {
        deck_name: String,
    },
    Suggest,
    List,
    Rename {
        from_name: String,
        to_name: String,
    },
    SetWildcards {
        common: u32,
        uncommon: u32,
        rare: u32,
        mythic: u32,
    },
    Booster,
    Which {
        query: String,
    },
}

fn export<P: AsRef<Path>>(deck_name: &str, roster: &Roster<P>) -> Result<()> {
    let deck = roster
        .iter()
        .find(|in_roster| in_roster.name == deck_name)
        .with_context(|| format!("Failed to find deck {deck_name} in roster"))?;
    clipboard_win::set_clipboard(clipboard_win::formats::Unicode, deck.to_string())
        .map_err(|err| anyhow!("Failed to set clipboard {err}"))?;
    Ok(())
}

fn missing<P: AsRef<Path>>(
    deck_name: &str,
    roster: &Roster<P>,
    inventory: &Inventory,
) -> Result<()> {
    let deck = roster
        .iter()
        .find(|in_cat| in_cat.name == deck_name)
        .ok_or(anyhow!("Cannot find deck {deck_name} in deck roster"))?;
    let missing_cards = inventory.missing_cards(deck);

    let mut missing_cards: Vec<_> = missing_cards.collect();
    missing_cards.sort_by_key(|m| m.rarity);
    let fold_missing_cards = |acc: (u8, u8, u8, u8), card: &CardData| match card.rarity {
        Rarity::Common => (card.amount + acc.0, acc.1, acc.2, acc.3),
        Rarity::Uncommon => (acc.0, card.amount + acc.1, acc.2, acc.3),
        Rarity::Rare => (acc.0, acc.1, card.amount + acc.2, acc.3),
        Rarity::Mythic => (acc.0, acc.1, acc.2, card.amount + acc.3),
        Rarity::Land => acc,
        Rarity::Unknown => {
            eprintln!("Warning: unknown card encountered ({})", card.name);
            acc
        }
    };
    let (missing_c, missing_u, missing_r, missing_m) =
        missing_cards.iter().fold((0, 0, 0, 0), fold_missing_cards);
    println!("Missing commons: {missing_c}, missing uncommons: {missing_u}, missing rares: {missing_r}, missing mythics: {missing_m}.\n");
    missing_cards
        .iter()
        .filter(|m| m.amount > 0)
        .for_each(|missing| {
            println!("{:?}\t {} {}", missing.rarity, missing.amount, missing.name);
        });
    Ok(())
}

fn suggest<P: AsRef<Path>>(roster: &Roster<P>, inventory: &Inventory) -> Result<()> {
    let mut sug_common = HashMap::new();
    let mut sug_uncommon = HashMap::new();
    let mut sug_rare = HashMap::new();
    let mut sug_mythic = HashMap::new();
    let decks: Vec<_> = roster
        .iter()
        .filter(|deck| inventory.missing_cards(deck).count() > 1)
        .collect();

    for deck in decks {
        for (deck_amount, card_name) in deck.cards() {
            let card_cost = inventory
                .card_cost(card_name)
                .context("When computing card cost")?;
            let missing = deck_amount.saturating_sub(
                inventory
                    .card_amount(card_name)
                    .context("When computing card amount")?,
            );
            let rarity = inventory
                .cheapest_rarity(card_name)
                .context("When computing rarity")?;
            let deck_cost = inventory
                .deck_cost(deck)
                .context("When computing deck cost")?;
            let selected_sugg = match rarity {
                Rarity::Common => &mut sug_common,
                Rarity::Uncommon => &mut sug_uncommon,
                Rarity::Rare => &mut sug_rare,
                Rarity::Mythic => &mut sug_mythic,
                _ => continue,
            };
            let sugg_coeff = card_cost * missing as f32 / deck_cost;

            *selected_sugg.entry(card_name).or_insert(0.0) += sugg_coeff;
        }
    }

    let suggestions = vec![
        (sug_common, "Common"),
        (sug_uncommon, "Uncommon"),
        (sug_rare, "Rare"),
        (sug_mythic, "Mythic Rare"),
    ];
    for (suggestions, rarity) in suggestions {
        println!("{}", rarity);
        let mut suggestions: Vec<(&String, f32)> = suggestions.into_iter().collect();
        suggestions.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        for (card_name, sugg_coeff) in suggestions.iter().take(10) {
            println!("{:.2} {}", sugg_coeff, card_name);
        }
        println!();
    }
    Ok(())
}

fn add_from_file<P: AsRef<Path>>(
    deck_paths: &Vec<String>,
    names: Option<&Vec<String>>,
    roster: &mut Roster<P>,
) -> Result<()> {
    let names_iter = match names {
        Some(names) => Left(names.iter().map(|s| s.as_str())),
        None => Right(std::iter::repeat("Unnamed").take(deck_paths.len())),
    };
    let decks: Vec<Deck> = deck_paths
        .iter()
        .zip(names_iter)
        .map(|(deck_path, name)| {
            let deck = fs::read_to_string(deck_path)
                .context("Failed to find decklist")?
                .parse::<Deck>()
                .context("Failed to parse decklist")?
                .name(name);
            Ok(deck)
        })
        .collect::<anyhow::Result<Vec<Deck>>>()?;
    for deck in decks {
        roster.add_deck(&deck);
    }
    Ok(())
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
    let wildcards_path = app_dir.join("wildcards.json");
    let mut roster = Roster::open(&roster_path)
        .with_context(|| format!("Failed to open deck roster with path {roster_path:?}"))?;
    let inventory = Inventory::open(&collection_path, &wildcards_path)?;
    match cli.command {
        Some(Commands::Booster) => {
            unimplemented!()
        }
        Some(Commands::Export { deck_name }) => export(&deck_name, &roster)?,
        Some(Commands::Which { query }) => {
            let re = Regex::new(&query)?;
            for deck in roster.iter() {
                for (_, card_name) in deck.cards() {
                    let card_name = card_name.to_lowercase();
                    if re.is_match(&card_name) {
                        println!("{}\t{}", deck.name, card_name);
                    }
                }
            }
        }
        Some(Commands::Show { deck_name }) => roster
            .iter()
            .find(|in_roster| deck_name == in_roster.name)
            .map(|deck| println!("{deck}"))
            .ok_or(anyhow!(
                "Could not find {deck_name} in roster {roster_path:?}"
            ))?,
        Some(Commands::Remove { deck_name }) => {
            roster
                .remove_deck(&deck_name)
                .context("Failed to remove deck")?;
        }
        Some(Commands::AddFromFile { deck_paths, names }) => {
            add_from_file(&deck_paths, names.as_ref(), &mut roster)?
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
        Some(Commands::Missing { deck_name }) => missing(&deck_name, &roster, &inventory)?,
        Some(Commands::UpdateCollection { path }) => {
            std::fs::copy(path, collection_path)?;
        }
        Some(Commands::Rename { from_name, to_name }) => {
            let deck = roster
                .iter_mut()
                .find(|in_roster| in_roster.name == from_name)
                .ok_or(anyhow!(
                    "Could not find {from_name} in roster {roster_path:?}"
                ))?;
            deck.name = to_name;
        }
        Some(Commands::Suggest) => {
            suggest(&roster, &inventory)?;
        }
        Some(Commands::List) => {
            let mut decks = roster
                .iter()
                .cloned()
                .map(|deck| (inventory.deck_cost(&deck).unwrap(), deck))
                .collect_vec();
            decks.sort_unstable_by(|(c1, _), (c2, _)| c1.partial_cmp(c2).unwrap());
            for (coeff, deck) in decks {
                println!("{coeff:.2}\t {}", deck.name);
            }
        }
        Some(Commands::SetWildcards {
            common,
            uncommon,
            rare,
            mythic,
        }) => {
            let wildcards = Wildcards {
                common,
                uncommon,
                rare,
                mythic,
            };
            fs::write(wildcards_path, serde_json::to_string(&wildcards)?)?;
        }
        None => {}
    }
    Ok(())
}
