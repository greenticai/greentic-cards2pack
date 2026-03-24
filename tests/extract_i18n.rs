use std::fs;
use std::path::PathBuf;

use assert_cmd::cargo::cargo_bin_cmd;
use tempfile::TempDir;

mod support;

fn translate_fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cards/translate")
}

#[test]
fn extract_i18n_produces_bundle() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    support::copy_fixture_cards(&translate_fixtures(), &cards_dir);

    let output = tmp.path().join("en.json");

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("extract-i18n")
        .arg("--input")
        .arg(&cards_dir)
        .arg("--output")
        .arg(&output)
        .assert()
        .success();

    assert!(output.is_file());
    let bundle: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&output).unwrap()).unwrap();
    let map = bundle.as_object().unwrap();

    // welcome.json has greentic.cardId = "welcome"
    assert!(map.keys().any(|k| k.starts_with("card.welcome.")));
    assert_eq!(
        map.get("card.welcome.body_0.text").and_then(|v| v.as_str()),
        Some("Welcome to Greentic!")
    );

    // form-input.json uses filename stem
    assert!(map.keys().any(|k| k.starts_with("card.form_input.")));

    // detail-card.json has FactSet titles
    assert!(
        map.keys()
            .any(|k| k.contains("facts") && k.ends_with(".title"))
    );
}

#[test]
fn extract_i18n_respects_prefix() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    support::copy_fixture_cards(&translate_fixtures(), &cards_dir);

    let output = tmp.path().join("en.json");

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("extract-i18n")
        .arg("--input")
        .arg(&cards_dir)
        .arg("--output")
        .arg(&output)
        .arg("--prefix")
        .arg("myapp")
        .assert()
        .success();

    let bundle: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&output).unwrap()).unwrap();
    let map = bundle.as_object().unwrap();

    assert!(
        map.keys().all(|k| k.starts_with("myapp.")),
        "all keys should start with custom prefix"
    );
}

#[test]
fn extract_i18n_skips_existing_patterns() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    support::copy_fixture_cards(&translate_fixtures(), &cards_dir);

    let output = tmp.path().join("en.json");

    // Default behavior: skip $t() patterns
    cargo_bin_cmd!("greentic-cards2pack")
        .arg("extract-i18n")
        .arg("--input")
        .arg(&cards_dir)
        .arg("--output")
        .arg(&output)
        .assert()
        .success();

    let bundle: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&output).unwrap()).unwrap();
    let map = bundle.as_object().unwrap();

    // detail-card.json has "$t(existing.key)" — should be skipped
    assert!(
        !map.values().any(|v| v.as_str() == Some("$t(existing.key)")),
        "$t() pattern should be skipped by default"
    );
}

#[test]
fn extract_i18n_includes_existing_when_flagged() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    support::copy_fixture_cards(&translate_fixtures(), &cards_dir);

    let output = tmp.path().join("en.json");

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("extract-i18n")
        .arg("--input")
        .arg(&cards_dir)
        .arg("--output")
        .arg(&output)
        .arg("--include-existing")
        .assert()
        .success();

    let bundle: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&output).unwrap()).unwrap();
    let map = bundle.as_object().unwrap();

    assert!(
        map.values().any(|v| v.as_str() == Some("$t(existing.key)")),
        "$t() pattern should be included with --include-existing"
    );
}

#[test]
fn extract_i18n_verbose_prints_report() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    support::copy_fixture_cards(&translate_fixtures(), &cards_dir);

    let output = tmp.path().join("en.json");

    let assert = cargo_bin_cmd!("greentic-cards2pack")
        .arg("extract-i18n")
        .arg("--input")
        .arg(&cards_dir)
        .arg("--output")
        .arg(&output)
        .arg("--verbose")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("I18n Extraction Report"),
        "verbose should print report"
    );
}
