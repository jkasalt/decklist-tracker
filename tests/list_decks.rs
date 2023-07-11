use assert_fs::NamedTempFile;
use decklist_tracker::{Catalogue, Deck};
#[test]
fn list_decks_works() {
    let temp_file = NamedTempFile::new("test_catalogue.json").unwrap();
    let mut catalogue = Catalogue::open(&temp_file).unwrap();
    catalogue.add_deck(
        &Deck::from_file("boros_turns.txt")
            .unwrap()
            .name("boros turns"),
    );
    catalogue.add_deck(
        &Deck::from_file("deification_prison.txt")
            .unwrap()
            .name("Deification prison"),
    );
    assert_eq!(
        catalogue.deck_list().collect::<Vec<_>>(),
        vec!["boros turns", "Deification prison"]
    );
}
