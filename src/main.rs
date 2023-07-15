use anyhow::{anyhow, Context};
use clap::{arg, Arg, ArgAction, Command, Parser, Subcommand};
use decklist_tracker::{Catalogue, Deck};
use directories::BaseDirs;
use std::{
    cell::OnceCell,
    fs::{self},
    path::PathBuf,
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, value_name = "CATALOGUE_PATH")]
    catalogue_path: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Add { deck_paths: Vec<String> },
    List,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let catalogue_path = BaseDirs::new()
        .ok_or(anyhow!("Could not obtain system's base directories"))?
        .data_dir()
        .join("/my_projects/decklist_tracker/catalogue.json");
    let catalogue_path = cli
        .catalogue_path
        .map(PathBuf::from)
        .unwrap_or(catalogue_path);
    match cli.command {
        Some(Commands::Add { deck_paths }) => {
            let decks: Vec<Deck> = deck_paths
                .iter()
                .map(|p| {
                    let deck = fs::read_to_string(p)
                        .context("Failed to find decklist")?
                        .parse()
                        .context("Failed to parse decklist")?;
                    Ok(deck)
                })
                .collect::<anyhow::Result<Vec<Deck>>>()?;
            let mut catalogue =
                Catalogue::open(catalogue_path).context("Failed to open deck catalogue")?;
            for deck in decks {
                catalogue.add_deck(&deck);
            }
        }
        Some(Commands::List) => {
            Catalogue::open(catalogue_path)
                .context("Failed to open deck catalogue")?
                .deck_list()
                .for_each(|name| println!("{name}"));
        }
        _ => unreachable!(),
    }
    Ok(())
}
