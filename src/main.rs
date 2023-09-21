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
    process::{Command, Stdio},
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(subcommand_required = true)]
struct Cli {
    #[arg(
        short,
        long,
        global = true,
        help = "What path to use for the deck roster json"
    )]
    roster_path: Option<PathBuf>,

    #[arg(
        short,
        long,
        global = true,
        help = "What path to use for the collection csv"
    )]
    collection_path: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        help = "Will ignore deck sideboards for calculations"
    )]
    ignore_sb: bool,

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
    Edit {
        deck_name: String,
    },
    Suggest {
        #[arg(
            long,
            short,
            help = "Will not favour cards from decks that are close to completion"
        )]
        equally: bool,
    },
    List,
    Rename {
        current_name: String,
        new_name: String,
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
    PrintCoeffs,
}

fn export<P: AsRef<Path>>(deck_name: &str, roster: &Roster<P>) -> Result<()> {
    let deck = roster.find(deck_name)?;
    clipboard_win::set_clipboard(clipboard_win::formats::Unicode, deck.to_string())
        .map_err(|err| anyhow!("Failed to set clipboard {err}"))?;
    Ok(())
}

fn missing<P: AsRef<Path>>(
    deck_name: &str,
    roster: &Roster<P>,
    inventory: &Inventory,
    ignore_sideboard: bool,
) -> Result<()> {
    let deck = roster.find(deck_name)?;
    let missing_cards = inventory.missing_cards(deck, ignore_sideboard);

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

fn suggest<P: AsRef<Path>>(
    roster: &Roster<P>,
    inventory: &Inventory,
    ignore_sideboard: bool,
    equally: bool,
) -> Result<()> {
    let mut sug_common = HashMap::new();
    let mut sug_uncommon = HashMap::new();
    let mut sug_rare = HashMap::new();
    let mut sug_mythic = HashMap::new();
    let decks: Vec<_> = roster
        .decks()
        .filter(|deck| inventory.missing_cards(deck, ignore_sideboard).count() > 1)
        .collect();

    for deck in decks {
        for (deck_amount, card_name) in deck.cards(ignore_sideboard) {
            let rarity = inventory
                .cheapest_rarity(card_name)
                .context("When computing rarity")?;
            let deck_cost = if !equally {
                inventory
                    .deck_cost(deck, ignore_sideboard)
                    .context("When computing deck cost")?
            } else {
                100.0
            };
            let selected_sugg = match rarity {
                Rarity::Common => &mut sug_common,
                Rarity::Uncommon => &mut sug_uncommon,
                Rarity::Rare => &mut sug_rare,
                Rarity::Mythic => &mut sug_mythic,
                _ => continue,
            };
            let sugg_coeff =
                inventory.card_cost_considering_deck(card_name, deck_amount)? / deck_cost;

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
        .unwrap_or_else(|| app_dir.join("roster.json"));
    let collection_path = cli
        .collection_path
        .unwrap_or_else(|| app_dir.join("collection.csv"));
    let wildcards_path = app_dir.join("wildcards.json");
    let mut roster = Roster::open(&roster_path)
        .with_context(|| format!("Failed to open deck roster with path {roster_path:?}"))?;
    let inventory = Inventory::open(&collection_path, &wildcards_path).with_context(|| {
        format!("Failed to open inventory with paths {collection_path:?}, and {wildcards_path:?}")
    })?;
    let ignore_sideboard = cli.ignore_sb;
    match cli.command {
        Some(Commands::AddFromFile { deck_paths, names }) => {
            add_from_file(&deck_paths, names.as_ref(), &mut roster)?
        }
        Some(Commands::Booster) => {
            // For each set, sums up the card values
            let mut set_value = HashMap::new();
            for deck in roster.decks() {
                for (&amount, card_name) in deck.cards(ignore_sideboard) {
                    let card_cheapest_version = inventory.cheapest_version(card_name)?;
                    let set_name = card_cheapest_version.set_name;
                    let card_cost = inventory.card_cost(card_name)?;
                    let missing_amount = amount.saturating_sub(inventory.card_amount(card_name)?);
                    *set_value.entry(set_name).or_insert(0.0) += card_cost * missing_amount as f32;
                }
            }
            println!("{set_value:#?}");
        }
        Some(Commands::Edit { deck_name }) => {
            let deck = roster.find(&deck_name)?;
            let tmp_file_name = "deck.tmp";
            fs::write(tmp_file_name, format!("{deck}").as_bytes())?;
            // TODO: make this work on machines that don't have nvim installed ?
            let mut child = Command::new("nvim")
                .arg(tmp_file_name)
                .stdin(Stdio::piped())
                .spawn()?;

            if !child.wait()?.success() {
                eprintln!("Vim exited with an error");
            }

            let modified_deck = fs::read_to_string(tmp_file_name)
                .context("When attempting to read temp file")?
                .parse::<Deck>()?
                .name(&deck_name);
            fs::remove_file(tmp_file_name)?;
            roster.replace(&deck_name, modified_deck)?;
        }
        Some(Commands::Export { deck_name }) => export(&deck_name, &roster)?,
        Some(Commands::List) => {
            let mut decks = roster
                .decks()
                .cloned()
                .map(|deck| (inventory.deck_cost(&deck, ignore_sideboard).unwrap(), deck))
                .collect_vec();
            decks.sort_unstable_by(|(c1, _), (c2, _)| c1.partial_cmp(c2).unwrap());
            for (coeff, deck) in decks {
                println!("{coeff:.2}\t {}", deck.name);
            }
        }
        Some(Commands::Missing { deck_name }) => {
            missing(&deck_name, &roster, &inventory, ignore_sideboard)?
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
        Some(Commands::PrintCoeffs) => println!("{:?}", inventory.wildcard_coeffs()),
        Some(Commands::Remove { deck_name }) => {
            roster
                .remove_deck(&deck_name)
                .context("Failed to remove deck")?;
        }
        Some(Commands::Rename {
            current_name,
            new_name,
        }) => {
            let deck = roster.find_mut(&current_name)?;
            deck.name = new_name;
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
        Some(Commands::Show { deck_name }) => {
            roster.find(&deck_name).map(|deck| println!("{deck}"))?
        }
        Some(Commands::Suggest { equally }) => {
            suggest(&roster, &inventory, ignore_sideboard, equally)?;
        }
        Some(Commands::UpdateCollection { path }) => {
            std::fs::copy(path, collection_path)?;
        }
        Some(Commands::Which { query }) => {
            let re = Regex::new(&query)?;
            for deck in roster.decks() {
                for (amount, card_name) in deck.cards(ignore_sideboard) {
                    if re.is_match(&card_name.to_lowercase()) {
                        println!("{}\t{amount} {card_name}", deck.name);
                    }
                }
            }
        }
        None => {}
    }
    Ok(())
}
