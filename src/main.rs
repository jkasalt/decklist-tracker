use anyhow::{anyhow, Context, Result};
use clap::{arg, Parser, Subcommand};
use detr::{CardData, Collection, Deck, Rarity, RefCardData, Roster, Wildcards};
use directories::BaseDirs;
use either::*;
use itertools::Itertools;
use regex::Regex;
use std::{
    collections::HashMap,
    fs::{self, File},
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

fn with_crafting_costs(
    decks: Vec<Deck>,
    collection: &Collection,
    wildcards: &Wildcards,
) -> Vec<(f32, Deck)> {
    let coeffs = wildcards.coefficients();
    let closeness_cap = 300.0;
    decks
        .into_iter()
        .map(|deck| {
            let val: f32 = collection
                .missing(&deck)
                .map(|card_data| f32::from(card_data.amount) * coeffs.select(&card_data.rarity))
                .sum();
            (val.powi(2) / closeness_cap, deck)
        })
        .collect()
}

fn sort_by_missing(decks: &mut [Deck], collection: &Collection, wildcards: &Wildcards) {
    let coeffs = wildcards.coefficients();
    decks.sort_unstable_by(|deck1, deck2| {
        let val1: f32 = collection
            .missing(deck1)
            .map(|card_data| f32::from(card_data.amount) * coeffs.select(&card_data.rarity))
            .sum();
        let val2 = collection
            .missing(deck2)
            .map(|card_data| f32::from(card_data.amount) * coeffs.select(&card_data.rarity))
            .sum();
        val1.partial_cmp(&val2).unwrap()
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

fn smart_amount(versions: Vec<RefCardData>, wildcards: &Wildcards) -> Result<CardData> {
    let card_amount = versions
        .iter()
        .map(|card_data| *card_data.amount)
        .sum::<u8>();
    if card_amount >= 4 {
        let ref_data = versions.get(0).ok_or(anyhow!("Empty card versions vec"))?;
        return Ok(CardData {
            amount: 4,
            ..ref_data.clone().to_owned()
        });
    }
    versions
        .iter()
        .min_by(|card_data1, card_data2| {
            let w1 = wildcards.select(card_data1.rarity);
            let w2 = wildcards.select(card_data2.rarity);
            ((4 - card_data1.amount) as f64 / w1 as f64)
                .partial_cmp(&((4 - card_data2.amount) as f64 / w2 as f64))
                .unwrap()
        })
        .map(|ref_data| ref_data.to_owned())
        .ok_or(anyhow!("Failed to find smart card amount for {versions:?}"))
}

fn suggest<P: AsRef<Path>>(
    roster: &Roster<P>,
    collection: &Collection,
    wildcards: &Wildcards,
) -> Result<HashMap<CardData, f64>> {
    let mut decks: Vec<_> = roster
        .iter()
        .filter(|deck| collection.missing(deck).count() > 1)
        .cloned()
        .collect();

    sort_by_missing(&mut decks, collection, wildcards);
    let decks = with_crafting_costs(decks, collection, wildcards);
    let mut suggestions = HashMap::new();

    let mut handle_card = |amount_deck: &u8, card_name, deck_coeff: f32| -> Result<()> {
        if let "Plains" | "Island" | "Swamp" | "Mountain" | "Forest" = card_name {
            return Ok(());
        }
        let card_group = collection
            .get(card_name)
            .ok_or(anyhow!("Failed to find card {card_name} in collection"))?;
        // For cards with multiple versions, get the one for which the wildcards cost is the least impactful
        let card_data = smart_amount(card_group, wildcards)
            .with_context(|| format!("When computing smart amount for {card_name}"))?;
        let needed = amount_deck.saturating_sub(card_data.amount);
        *suggestions.entry(card_data).or_default() += needed as f64 / deck_coeff as f64;
        Ok(())
    };
    for (coeff, deck) in decks.iter() {
        for (amount_deck, card_name) in deck.cards() {
            handle_card(amount_deck, card_name, *coeff)?;
        }
    }
    Ok(suggestions)
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
    let wildcards: Wildcards = if !wildcards_path.exists() {
        Wildcards::default()
    } else {
        serde_json::from_reader(File::open(&wildcards_path)?).unwrap_or_default()
    };
    let mut roster = Roster::open(&roster_path)
        .with_context(|| format!("Failed to open deck roster with path {roster_path:?}"))?;
    let collection = Collection::from_csv(&collection_path)
        .with_context(|| format!("Failed to open collection with path {collection_path:?}"))?;
    match cli.command {
        Some(Commands::Booster) => {
            // Get suggestion coeffs
            let sugg_coeffs = suggest(&roster, &collection, &wildcards)?;
            let mut set_sugg = HashMap::new();
            for (card_data, coeff) in sugg_coeffs {
                *set_sugg.entry(card_data.set_name).or_insert(0.0) += coeff;
            }
            let mut set_sugg = set_sugg.iter().collect_vec();
            set_sugg
                .sort_unstable_by(|(_, coeff1), (_, coeff2)| coeff2.partial_cmp(coeff1).unwrap());
            println!("{set_sugg:#?}");
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
        Some(Commands::Suggest) => {
            let sugg_coeffs = suggest(&roster, &collection, &wildcards)?;
            let mut sug_common = HashMap::new();
            let mut sug_uncommon = HashMap::new();
            let mut sug_rare = HashMap::new();
            let mut sug_mythic = HashMap::new();
            for (card_data, coeff) in sugg_coeffs.iter() {
                let selected_sugg = match card_data.rarity {
                    Rarity::Common => &mut sug_common,
                    Rarity::Uncommon => &mut sug_uncommon,
                    Rarity::Rare => &mut sug_rare,
                    Rarity::Mythic => &mut sug_mythic,
                    _ => continue,
                };
                *selected_sugg.entry(card_data).or_insert(0.0) += coeff;
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
                for (card_data, amount) in suggestion_group.iter().take(20) {
                    println!("{:.2} {}", amount * 1e3, card_data.name);
                }
                println!();
            }
        }
        Some(Commands::List) => {
            let mut decks: Vec<_> = roster.iter().cloned().collect();
            sort_by_missing(&mut decks, &collection, &wildcards);
            for (coeff, deck) in with_crafting_costs(decks, &collection, &wildcards) {
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
