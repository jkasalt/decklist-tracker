use assert_fs::NamedTempFile;
use decklist_tracker::{Deck, Roster};
#[test]
fn list_decks_works() {
    let temp_file = NamedTempFile::new("test_roster.json").unwrap();
    let mut roster = Roster::open(&temp_file).unwrap();
    roster.add_deck(
        &Deck::from_file("boros_turns.txt")
            .unwrap()
            .name("boros turns"),
    );
    roster.add_deck(
        &Deck::from_file("deification_prison.txt")
            .unwrap()
            .name("Deification prison"),
    );
    assert_eq!(
        roster.deck_list().collect::<Vec<_>>(),
        vec!["boros turns", "Deification prison"]
    );
}
