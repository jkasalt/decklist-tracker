// use anyhow::{Context, Result};
// use clap::{arg, Command};
// use decklist_tracker::{Collection, Deck};
// use std::{
//     fs::{self, File},
//     io::{BufReader, Write},
// };

// fn cli() -> Command {
//     Command::new("deck")
//         .about("Manipulate decklists")
//         .subcommand_required(true)
//         .arg_required_else_help(true)
//         .allow_external_subcommands(true)
//         .subcommand(
//             Command::new("add")
//                 .about("Add decklist")
//                 .arg(arg!(<PATH> "The filepath of the decklist to add"))
//                 .arg_required_else_help(true),
//         )
// }

fn main() -> anyhow::Result<()> {
    // let deck: Deck = fs::read_to_string("deification_prison.txt")?
    //     .parse()
    //     .context("Failed to parse deck")?;
    // let collection = Collection::from_csv("collection.csv")?;

    // let matches = cli().get_matches();
    // match matches.subcommand() {
    //     Some(("add", sub_matches)) => {
    //         let deck_path = sub_matches
    //             .get_one::<String>("PATH")
    //             .expect("A path should be provided");
    //         let deck: Deck = fs::read_to_string(deck_path)?.parse()?;
    //         let mut catalogue = File::open("catalogue.json")?;
    //         let reader = BufReader::new(&catalogue);
    //         let mut decks: Vec<Deck> = serde_json::from_reader(reader)?;
    //         decks.push(deck);
    //         catalogue.write_all(serde_json::to_string(&decks)?.as_bytes())?;
    //     }
    //     _ => unreachable!(),
    // }
    Ok(())
}
