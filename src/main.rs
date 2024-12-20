use anyhow::{anyhow, Context, Result};
use clap::{arg, Parser, Subcommand};
use detr::{
    card_getter::CardGetter, collection::Collection, craft_suggester::CraftRecommender,
    mtga_id_translator::MtgaIdTranslator, Deck, Inventory, Rarity, Roster, Wildcards,
};
use directories::BaseDirs;
use either::{Left, Right};
use itertools::Itertools;
use mktemp::Temp;
use regex::Regex;
use std::{
    collections::HashMap,
    fs::{self},
    path::PathBuf,
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
        help = "What path to use for the collection json"
    )]
    collection_path: Option<PathBuf>,

    #[arg(
        short,
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
    #[command(alias = "u")]
    UpdateCollection,
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
    #[command(alias = "s")]
    Suggest {
        #[arg(
            long,
            short,
            help = "Will not favour cards from decks that are close to completion"
        )]
        equally: bool,
    },
    #[command(alias = "l")]
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
    WhichSet {
        set: String,
    },
    Recommend {
        rare_limit: usize,
        mythic_limit: usize,
        #[arg(long, short, help = "Result will contain the specified decks")]
        with: Option<Vec<String>>,
    },
    PrintCoeffs,
}

fn export(deck_name: &str, roster: &Roster) -> Result<()> {
    let deck = roster.find(deck_name)?;
    clipboard_win::set_clipboard(clipboard_win::formats::Unicode, deck.to_string())
        .map_err(|err| anyhow!("Failed to set clipboard {err}"))?;
    Ok(())
}

fn missing(
    deck_name: &str,
    roster: &Roster,
    inventory: &Inventory,
    ignore_sideboard: bool,
) -> Result<()> {
    let deck = roster.find(deck_name)?;
    let missing_cards = inventory.missing_cards(deck, ignore_sideboard);

    let mut missing_cards = missing_cards?;
    missing_cards.sort_by_key(|m| m.2);
    let fold_missing_cards =
        |acc: (u8, u8, u8, u8), card: &(&String, u8, Rarity, &String)| match card.2 {
            Rarity::Common => (card.1 + acc.0, acc.1, acc.2, acc.3),
            Rarity::Uncommon => (acc.0, card.1 + acc.1, acc.2, acc.3),
            Rarity::Rare => (acc.0, acc.1, card.1 + acc.2, acc.3),
            Rarity::Mythic => (acc.0, acc.1, acc.2, card.1 + acc.3),
            Rarity::Land => acc,
            Rarity::Unknown => {
                eprintln!("Warning: unknown card encountered ({})", card.0);
                acc
            }
        };
    let (missing_c, missing_u, missing_r, missing_m) =
        missing_cards.iter().fold((0, 0, 0, 0), fold_missing_cards);
    println!("Missing commons: {missing_c}, missing uncommons: {missing_u}, missing rares: {missing_r}, missing mythics: {missing_m}.\n");
    missing_cards
        .iter()
        .filter(|m| m.1 > 0)
        .for_each(|missing| {
            println!("{:?}\t {} {}", missing.2, missing.1, missing.0);
        });
    Ok(())
}

fn suggest(
    roster: &Roster,
    inventory: &mut Inventory,
    ignore_sideboard: bool,
    equally: bool,
) -> Result<()> {
    let mut sug_common = HashMap::new();
    let mut sug_uncommon = HashMap::new();
    let mut sug_rare = HashMap::new();
    let mut sug_mythic = HashMap::new();

    for deck in roster.decks() {
        for (card_name, deck_amount) in deck.cards(ignore_sideboard) {
            let rarity = inventory
                .cheapest_rarity(card_name)
                .context("When computing rarity")?;
            let deck_cost = if equally {
                100.0
            } else {
                inventory
                    .deck_cost(deck, ignore_sideboard)
                    .with_context(|| format!("Failed to compute deck cost for `{}`", deck.name))?
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
        println!("{rarity}");
        let mut suggestions: Vec<(&String, f32)> = suggestions.into_iter().collect();
        suggestions.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        for (card_name, sugg_coeff) in suggestions.iter().take(10) {
            println!("{sugg_coeff:.2} {card_name}");
        }
        println!();
    }
    Ok(())
}

fn add_from_file(
    deck_paths: &[String],
    names: Option<&Vec<String>>,
    roster: &mut Roster,
) -> Result<()> {
    let names_iter = match names {
        Some(names) => Left(names.iter().map(std::string::String::as_str)),
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
        roster.add_deck(deck);
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
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
        .unwrap_or_else(|| app_dir.join("collection.json"));
    let wildcards_path = app_dir.join("wildcards.json");
    let mut translator = MtgaIdTranslator::load_from_file(app_dir.join("translator.ron"))
        .context("Failed to load translator.json file")?;
    let mut roster = Roster::open(&roster_path)
        .with_context(|| format!("Failed to open deck roster with path {roster_path:?}"))?;
    let mut inventory = Inventory::open(&collection_path, &wildcards_path).with_context(|| {
        format!("Failed to open inventory with paths {collection_path:?}, and {wildcards_path:?}")
    })?;
    let ignore_sideboard = cli.ignore_sb;
    match cli.command {
        Some(Commands::AddFromFile { deck_paths, names }) => {
            add_from_file(&deck_paths, names.as_ref(), &mut roster)?;
        }
        Some(Commands::Booster) => {
            // For each set, sums up the card values
            let mut set_values = HashMap::new();
            for (card_name, amount) in roster.cards(ignore_sideboard) {
                let card_cheapest_version = inventory.cheapest_version(card_name)?;
                let set_name = &card_cheapest_version.2;
                let card_cost = inventory.card_cost(card_name)?;
                let missing_amount = amount.saturating_sub(inventory.card_amount(card_name)?);
                *set_values.entry(set_name).or_insert(0.0) += card_cost * f32::from(missing_amount);
            }
            let mut set_values = set_values.iter().collect_vec();
            set_values.sort_unstable_by(|(_, v1), (_, v2)| v2.partial_cmp(v1).unwrap());
            set_values
                .iter()
                .take(10)
                .for_each(|(set, value)| println!("{value} {set}"));
        }
        Some(Commands::Edit { deck_name }) => {
            let deck = roster.find(&deck_name)?;
            let tmp_file = Temp::new_file()?;
            fs::write(&tmp_file, format!("{deck}").as_bytes())?;
            // TODO: make this work on machines that don't have nvim installed ?
            let mut child = Command::new("nvim")
                .arg(tmp_file.to_path_buf())
                .stdin(Stdio::piped())
                .spawn()?;

            if !child.wait()?.success() {
                eprintln!("Vim exited with an error");
            }

            let modified_deck = fs::read_to_string(tmp_file)
                .context("When attempting to read temp file")?
                .parse::<Deck>()?
                .name(&deck_name);
            roster.replace(&deck_name, modified_deck)?;
        }
        Some(Commands::Export { deck_name }) => export(&deck_name, &roster)?,
        Some(Commands::List) => {
            let costs = roster
                .decks()
                .map(|deck| {
                    (inventory.deck_cost(deck, ignore_sideboard))
                        .with_context(|| format!("Failed to compute deck cost for `{}`", deck.name))
                })
                .collect::<Result<Vec<_>>>()?; // Just collect here, to make error-handling less of a headache
            let mut decks = costs.iter().zip(roster.decks()).collect_vec();
            decks.sort_unstable_by(|(c1, _), (c2, _)| c1.partial_cmp(c2).unwrap());
            for (coeff, deck) in decks {
                println!("{coeff:.2}\t {}", deck.name);
            }
        }
        Some(Commands::Missing { deck_name }) => {
            missing(&deck_name, &roster, &inventory, ignore_sideboard)?;
        }
        Some(Commands::Paste { name }) => {
            let deck: Deck =
                clipboard_win::get_clipboard::<String, _>(clipboard_win::formats::Unicode)
                    .map_err(|err| anyhow!("Failed to read clipboard: {err}"))?
                    .parse::<Deck>()
                    .context("Failed to parse deck from clipboard")?
                    .name(&name);
            roster.add_deck(deck);
        }
        Some(Commands::PrintCoeffs) => println!("{:?}", inventory.wildcard_coeffs()),
        Some(Commands::Recommend {
            rare_limit,
            mythic_limit,
            with,
        }) => {
            let collection = Collection::open(&collection_path)?;
            let craft_suggester = CraftRecommender::new(
                rare_limit,
                mythic_limit,
                ignore_sideboard,
                with,
                &roster,
                &collection,
            );
            let result = craft_suggester.recommend();
            println!("{result:#?}");
        }
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
                common: common as f32,
                uncommon: uncommon as f32,
                rare: rare as f32,
                mythic: mythic as f32,
            };
            fs::write(wildcards_path, serde_json::to_string(&wildcards)?)?;
        }
        Some(Commands::Show { deck_name }) => {
            roster.find(&deck_name).map(|deck| println!("{deck}"))?;
        }
        Some(Commands::Suggest { equally }) => {
            suggest(&roster, &mut inventory, ignore_sideboard, equally)?;
        }
        Some(Commands::UpdateCollection) => {
            // std::fs::copy(path, collection_path)?;
            let recently_fetched =
                CardGetter::owned_cards(&mut translator).context("Failed to get owned cards")?;
            inventory.update_collection(recently_fetched, &roster);
        }
        Some(Commands::Which { query }) => {
            let re = Regex::new(&query)?;
            for deck in roster.decks() {
                for (card_name, amount) in deck.cards(ignore_sideboard) {
                    if re.is_match(&card_name.to_lowercase()) {
                        println!("{}\t{amount} {card_name}", deck.name);
                    }
                }
            }
        }
        Some(Commands::WhichSet { set: set_name }) => {
            let mut found_cards = HashMap::new();
            for (card_name, amount) in roster.cards(ignore_sideboard) {
                let card = inventory.cheapest_version(card_name)?;
                if card.2 == set_name {
                    let missing_amount = amount.saturating_sub(inventory.card_amount(card_name)?);
                    *found_cards.entry(card_name).or_insert(0) += missing_amount;
                }
            }
            let mut found_cards = found_cards
                .into_iter()
                .filter(|(_, missing_amount)| *missing_amount > 0)
                .collect_vec();
            found_cards.sort_unstable_by_key(|(_, amount)| std::cmp::Reverse(*amount));
            println!(
                "Found a total of {} missing cards in {set_name}\n",
                found_cards.len()
            );
            for (card, amount) in found_cards {
                println!("{amount} {card}");
            }
        }
        None => {}
    }
    Ok(())
}
