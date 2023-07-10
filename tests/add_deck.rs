use std::fs::read_to_string;
use assert_fs::NamedTempFile;
use decklist_tracker::{Catalogue, Deck};

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
    catalogue
        .add_deck(&decklist)
        .unwrap_or_else(|err| panic!("ERROR: failed to add deck to catalogue because {err}"));
    drop(catalogue);
    let result = read_to_string(temp_file)
        .unwrap_or_else(|err| panic!("ERROR: Failed to read temp file because {err}"));
    let mut expected = String::from('[');
    expected.push_str(&serde_json::to_string(&decklist).unwrap());
    expected.push(']');
    assert_eq!(result, expected);
}
