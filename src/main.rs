use anyhow::Context;
use clap::{arg, Command};
use decklist_tracker::{Catalogue, Deck, CATALOGUE_PATH};
use std::fs::{self};

fn cli() -> Command {
    Command::new("deck")
        .about("Manipulate decklists")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .allow_external_subcommands(true)
        .subcommand(
            Command::new("add")
                .about("Add decklist")
                .arg(arg!(<PATH> "The filepath of the decklist to add"))
                .arg_required_else_help(true),
        )
        .subcommand(Command::new("list").about("List all decks"))
}

fn main() -> anyhow::Result<()> {
    let matches = cli().get_matches();
    match matches.subcommand() {
        Some(("add", sub_matches)) => {
            let deck_path = sub_matches
                .get_one::<String>("PATH")
                .expect("A path should be provided");
            let deck: Deck = fs::read_to_string(deck_path)
                .context("Failed to find decklist")?
                .parse()
                .context("Failed to parse decklist")?;
            let mut catalogue =
                Catalogue::open(CATALOGUE_PATH).context("Failed to open deck catalogue")?;
            catalogue.add_deck(&deck);
        }
        Some(("list", _)) => {
            Catalogue::open(CATALOGUE_PATH)
                .context("Failed to open deck catalogue")?
                .deck_list()
                .for_each(|name| println!("{name}"));
        }
        _ => unreachable!(),
    }
    Ok(())
}
