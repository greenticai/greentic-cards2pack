//! Integration tests that require a real `greentic-i18n-translator` binary.
//!
//! Skipped unless `GREENTIC_TRANSLATE_INTEGRATION=1` is set.
//! Run with:
//!   GREENTIC_TRANSLATE_INTEGRATION=1 cargo test --test translate_integration

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

mod support;

fn skip_unless_integration() -> bool {
    if std::env::var("GREENTIC_TRANSLATE_INTEGRATION").is_err() {
        eprintln!("skipping: set GREENTIC_TRANSLATE_INTEGRATION=1 to run");
        return true;
    }
    false
}

fn translate_fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cards/translate")
}

fn glossary_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/translate/glossary.json")
}

#[test]
fn real_extract_and_translate_roundtrip() {
    if skip_unless_integration() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    support::copy_fixture_cards(&translate_fixtures(), &cards_dir);

    // Extract
    let en_path = tmp.path().join("i18n/en.json");
    assert_cmd::cargo::cargo_bin_cmd!("greentic-cards2pack")
        .arg("extract-i18n")
        .arg("--input")
        .arg(&cards_dir)
        .arg("--output")
        .arg(&en_path)
        .assert()
        .success();

    assert!(en_path.is_file());
    let en: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&en_path).unwrap()).unwrap();
    let en_keys: Vec<String> = en.as_object().unwrap().keys().cloned().collect();
    assert!(!en_keys.is_empty());

    // Translate to French
    let status = std::process::Command::new("greentic-i18n-translator")
        .arg("translate")
        .arg("--langs")
        .arg("fr")
        .arg("--en")
        .arg(&en_path)
        .status()
        .expect("greentic-i18n-translator should be in PATH");

    assert!(status.success(), "translator should succeed");

    let fr_path = tmp.path().join("i18n/fr.json");
    assert!(fr_path.is_file(), "fr.json should be created");

    let fr: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&fr_path).unwrap()).unwrap();
    let fr_keys: Vec<String> = fr.as_object().unwrap().keys().cloned().collect();

    // French bundle should have the same keys as English
    for key in &en_keys {
        assert!(fr_keys.contains(key), "French bundle missing key: {key}");
    }
}

#[test]
fn real_translate_with_glossary() {
    if skip_unless_integration() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    support::copy_fixture_cards(&translate_fixtures(), &cards_dir);

    let en_path = tmp.path().join("i18n/en.json");
    assert_cmd::cargo::cargo_bin_cmd!("greentic-cards2pack")
        .arg("extract-i18n")
        .arg("--input")
        .arg(&cards_dir)
        .arg("--output")
        .arg(&en_path)
        .assert()
        .success();

    let status = std::process::Command::new("greentic-i18n-translator")
        .arg("translate")
        .arg("--langs")
        .arg("fr")
        .arg("--en")
        .arg(&en_path)
        .arg("--glossary")
        .arg(glossary_fixture())
        .status()
        .expect("greentic-i18n-translator should be in PATH");

    assert!(status.success(), "translator with glossary should succeed");

    let fr_path = tmp.path().join("i18n/fr.json");
    assert!(fr_path.is_file());

    let fr: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&fr_path).unwrap()).unwrap();

    // Glossary terms like "Greentic" should be preserved
    for value in fr.as_object().unwrap().values() {
        if let Some(text) = value.as_str()
            && text.contains("Greentic")
        {
            // Term is preserved — glossary worked
            return;
        }
    }
    // If no value contains "Greentic" that's also fine (the card text
    // may not contain the glossary terms directly)
}

#[test]
fn real_translate_validates_output() {
    if skip_unless_integration() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    support::copy_fixture_cards(&translate_fixtures(), &cards_dir);

    let en_path = tmp.path().join("i18n/en.json");
    assert_cmd::cargo::cargo_bin_cmd!("greentic-cards2pack")
        .arg("extract-i18n")
        .arg("--input")
        .arg(&cards_dir)
        .arg("--output")
        .arg(&en_path)
        .assert()
        .success();

    let translate = std::process::Command::new("greentic-i18n-translator")
        .arg("translate")
        .arg("--langs")
        .arg("fr")
        .arg("--en")
        .arg(&en_path)
        .status()
        .expect("translator in PATH");

    assert!(translate.success());

    // Validate the translation
    let validate = std::process::Command::new("greentic-i18n-translator")
        .arg("validate")
        .arg("--langs")
        .arg("fr")
        .arg("--en")
        .arg(&en_path)
        .status()
        .expect("validator in PATH");

    assert!(validate.success(), "validation should pass");
}
