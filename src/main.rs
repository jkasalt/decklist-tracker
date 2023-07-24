use anyhow::{anyhow, bail, Context, Result};
use clap::{arg, Parser, Subcommand};
use detr::{CardData, Collection, Deck, Rarity, Roster};
use directories::BaseDirs;
use either::*;
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
}

fn sort_by_missing(decks: &mut [Deck], collection: &Collection) {
    decks.sort_unstable_by_key(|deck| {
        collection
            .missing(deck)
            .filter(|card_data| matches!(card_data.rarity, Rarity::Common | Rarity::Rare))
            .map(|card| card.amount)
            .sum::<u8>()
    });
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
    collection: &Collection,
) -> Result<()> {
    let missing_cards = roster
        .iter()
        .find(|in_cat| in_cat.name == deck_name)
        .map(|deck| collection.missing(deck))
        .ok_or(anyhow!("Cannot find deck {deck_name} in deck roster"))?;

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

fn suggest<P: AsRef<Path>>(roster: &Roster<P>, collection: Collection) -> Result<()> {
    let mut sug_common = HashMap::new();
    let mut sug_uncommon = HashMap::new();
    let mut sug_rare = HashMap::new();
    let mut sug_mythic = HashMap::new();

    let mut decks: Vec<_> = roster
        .iter()
        .filter(|deck| collection.missing(deck).count() > 1)
        .cloned()
        .collect();
    sort_by_missing(&mut decks, &collection);
    let collection = collection.into_hash_map();
    let mut handle_card = |amount_deck: &u8, card_name, i: usize| {
        if let "Plains" | "Island" | "Swamp" | "Mountain" | "Forest" = card_name {
            return;
        }
        let (amount_coll, rarity) = collection.get(card_name).unwrap_or(&(0, Rarity::Unknown));
        let needed = amount_deck.saturating_sub(*amount_coll);
        if needed == 0 {
            return;
        }
        let sugg_coeff = needed as f64 / (i + 1) as f64;
        let selected_sug = match rarity {
            Rarity::Common => &mut sug_common,
            Rarity::Uncommon => &mut sug_uncommon,
            Rarity::Rare => &mut sug_rare,
            Rarity::Mythic => &mut sug_mythic,
            Rarity::Unknown => {
                eprintln!("Warning: unknown card encountered ({card_name})");
                return;
            }
            Rarity::Land => return,
        };
        *selected_sug.entry(card_name).or_insert(0.0) += sugg_coeff;
    };

    for (i, deck) in decks.iter().enumerate() {
        for (amount_deck, card_name) in deck.cards() {
            handle_card(amount_deck, card_name, i);
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
        suggestion_group.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap()); // TODO: remove unwrap?, note: ascending order
        for (name, amount) in suggestion_group.iter().take(10) {
            println!("{amount:.2} {name}");
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
    let mut roster = Roster::open(&roster_path)
        .with_context(|| format!("Failed to open deck roster with path {roster_path:?}"))?;
    let collection = Collection::from_csv(&collection_path)
        .with_context(|| format!("Failed to open collection with path {collection_path:?}"))?;
    match cli.command {
        Some(Commands::Export { deck_name }) => export(&deck_name, &roster)?,
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
        Some(Commands::Missing { deck_name }) => missing(&deck_name, &roster, &collection)?,
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
        Some(Commands::Suggest) => suggest(&roster, collection)?,
        Some(Commands::List) => {
            let mut decks: Vec<_> = roster.iter().cloned().collect();
            sort_by_missing(&mut decks, &collection);
            for deck in decks {
                println!("{}", deck.name);
            }
        }
        None => {}
    }
    Ok(())
}
