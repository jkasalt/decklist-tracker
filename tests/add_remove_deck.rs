use anyhow::Result;
use assert_cmd::Command;
use assert_fs::NamedTempFile;
use decklist_tracker::{Deck, Roster};
use std::fs::read_to_string;

#[test]
fn add_should_produce_correct_json_if_no_file_is_present() {
    let temp_file = NamedTempFile::new("test_roster.json")
        .unwrap_or_else(|err| panic!("ERROR: failed to open temp file because {err}"));
    let roster = Roster::open(&temp_file)
        .unwrap_or_else(|err| panic!("ERROR: failed to open roster because {err}"));
    drop(roster);
    let result = read_to_string(temp_file)
        .unwrap_or_else(|err| panic!("ERROR: Failed to read temp file because {err}"));
    assert_eq!(&result, "[]");
}

#[test]
fn adding_should_produce_correct_file() -> Result<()> {
    let temp_file = NamedTempFile::new("test_roster.json")?;
    let mut command = Command::cargo_bin("decklist-tracker")?;
    command
        .arg("-r")
        .arg(temp_file.path())
        .arg("add-from-file")
        .arg("boros_turns.txt");
    let decklist: Deck = include_str!("../boros_turns.txt").parse()?;
    command.assert().success();
    let result = read_to_string(temp_file)?;
    let expected = serde_json::to_string(&[decklist])?;
    assert_eq!(result, expected);
    Ok(())
}

#[test]
fn adding_two_at_a_time() -> Result<()> {
    let temp_file = NamedTempFile::new("test_roster.json")?;
    let mut command = Command::cargo_bin("decklist-tracker")?;
    command
        .arg("-r")
        .arg(temp_file.path())
        .arg("add-from-file")
        .arg("boros_turns.txt")
        .arg("deification_prison.txt");
    let deck1: Deck = include_str!("../boros_turns.txt").parse()?;
    let deck2: Deck = include_str!("../deification_prison.txt").parse()?;
    command.assert().success();
    let result = read_to_string(temp_file)?;
    let expected = serde_json::to_string(&[deck1, deck2])?;
    assert_eq!(result, expected);
    Ok(())
}

#[test]
fn remove_deck_should_produce_correct_file_when_deck_is_present() {
    // setup
    let temp_file = NamedTempFile::new("test_roster.json").unwrap();
    let mut roster = Roster::open(&temp_file)
        .unwrap_or_else(|err| panic!("ERROR: failed to open roster because {err}"));
    roster.add_deck(
        &Deck::from_file("boros_turns.txt")
            .unwrap()
            .name("boros turns"),
    );
    let deck2 = Deck::from_file("deification_prison.txt")
        .unwrap()
        .name("Deification prison");
    roster.add_deck(&deck2);
    roster.write().unwrap();

    // do the thing
    roster.remove_deck("boros turns").unwrap();
    roster.write().unwrap();

    // check
    let result = read_to_string(&temp_file).unwrap();
    let expected = serde_json::to_string(&[deck2]).unwrap();
    assert_eq!(result, expected);
}
