use assert_fs::NamedTempFile;
use decklist_tracker::{Catalogue, Deck};
use std::fs::read_to_string;

#[test]
fn add_should_produce_correct_json_if_no_file_is_present() {
    let temp_file = NamedTempFile::new("test_catalogue.json")
        .unwrap_or_else(|err| panic!("ERROR: failed to open temp file because {err}"));
    let catalogue = Catalogue::open(&temp_file)
        .unwrap_or_else(|err| panic!("ERROR: failed to open catalogue because {err}"));
    drop(catalogue);
    let result = read_to_string(temp_file)
        .unwrap_or_else(|err| panic!("ERROR: Failed to read temp file because {err}"));
    assert_eq!(&result, "[]");
}

#[test]
fn adding_should_produce_correct_file() {
    let decklist: Deck = include_str!("../boros_turns.txt")
        .parse()
        .unwrap_or_else(|e| panic!("Failed to parse decklist: {e}"));
    let temp_file = NamedTempFile::new("test_catalogue.json")
        .unwrap_or_else(|err| panic!("ERROR: failed to open temp file because {err}"));
    let mut catalogue = Catalogue::open(&temp_file)
        .unwrap_or_else(|err| panic!("ERROR: failed to open catalogue because {err}"));
    catalogue.add_deck(&decklist);
    drop(catalogue);
    let result = read_to_string(temp_file)
        .unwrap_or_else(|err| panic!("ERROR: Failed to read temp file because {err}"));
    let mut expected = String::from('[');
    expected.push_str(&serde_json::to_string(&decklist).unwrap());
    expected.push(']');
    assert_eq!(result, expected);
}

#[test]
fn remove_deck_should_produce_correct_file_when_deck_is_present() {
    // setup
    let temp_file = NamedTempFile::new("test_catalogue.json").unwrap();
    let mut catalogue = Catalogue::open(&temp_file)
        .unwrap_or_else(|err| panic!("ERROR: failed to open catalogue because {err}"));
    catalogue.add_deck(
        &Deck::from_file("boros_turns.txt")
            .unwrap()
            .name("boros turns"),
    );
    let deck2 = Deck::from_file("deification_prison.txt")
        .unwrap()
        .name("Deification prison");
    catalogue.add_deck(&deck2);
    catalogue.write().unwrap();

    // do the thing
    catalogue.remove_deck("boros turns").unwrap();
    catalogue.write().unwrap();

    // check
    let result = read_to_string(&temp_file).unwrap();
    let expected = serde_json::to_string(&[deck2]).unwrap();
    assert_eq!(result, expected);
}
