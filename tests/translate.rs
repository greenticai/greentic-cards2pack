use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::TempDir;

mod support;

fn translate_fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cards/translate")
}

fn glossary_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/translate/glossary.json")
}

fn write_card(root: &Path, rel: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(
        path,
        r#"{
  "type": "AdaptiveCard",
  "body": [{ "type": "TextBlock", "text": "Hello World" }],
  "actions": [{ "type": "Action.Submit", "title": "OK", "data": {} }]
}
"#,
    )
    .unwrap();
}

fn setup_generate(tmp: &TempDir) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let cards_dir = tmp.path().join("cards");
    let out_dir = tmp.path().join("workspace");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&cards_dir).unwrap();
    fs::create_dir_all(&bin_dir).unwrap();

    support::copy_fixture_cards(&translate_fixtures(), &cards_dir);
    let greentic_pack = support::create_fake_greentic_pack(&bin_dir);

    (cards_dir, out_dir, bin_dir, greentic_pack)
}

#[test]
fn generate_auto_translate_creates_i18n_bundles() {
    let tmp = TempDir::new().unwrap();
    let (cards_dir, out_dir, bin_dir, greentic_pack) = setup_generate(&tmp);
    let translator = support::create_fake_i18n_translator(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--greentic-pack-bin")
        .arg(&greentic_pack)
        .arg("--auto-translate")
        .arg("--langs")
        .arg("fr,de")
        .env("GREENTIC_I18N_TRANSLATOR_BIN", &translator)
        .assert()
        .success();

    let i18n_dir = out_dir.join("assets/i18n");
    assert!(i18n_dir.join("en.json").is_file(), "en.json should exist");
    assert!(i18n_dir.join("fr.json").is_file(), "fr.json should exist");
    assert!(i18n_dir.join("de.json").is_file(), "de.json should exist");

    // Verify English bundle has keys
    let en: Value =
        serde_json::from_str(&fs::read_to_string(i18n_dir.join("en.json")).unwrap()).unwrap();
    assert!(!en.as_object().unwrap().is_empty());
}

#[test]
fn generate_auto_translate_default_languages() {
    let tmp = TempDir::new().unwrap();
    let (cards_dir, out_dir, bin_dir, greentic_pack) = setup_generate(&tmp);
    let translator = support::create_fake_i18n_translator(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--greentic-pack-bin")
        .arg(&greentic_pack)
        .arg("--auto-translate")
        .env("GREENTIC_I18N_TRANSLATOR_BIN", &translator)
        .assert()
        .success();

    let i18n_dir = out_dir.join("assets/i18n");
    // Default: fr, de, es, ja, zh
    for lang in &["fr", "de", "es", "ja", "zh"] {
        assert!(
            i18n_dir.join(format!("{lang}.json")).is_file(),
            "{lang}.json should be created by default"
        );
    }
}

#[test]
fn generate_auto_translate_with_glossary() {
    let tmp = TempDir::new().unwrap();
    let (cards_dir, out_dir, bin_dir, greentic_pack) = setup_generate(&tmp);
    let translator = support::create_fake_i18n_translator(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--greentic-pack-bin")
        .arg(&greentic_pack)
        .arg("--auto-translate")
        .arg("--langs")
        .arg("fr")
        .arg("--glossary")
        .arg(glossary_fixture())
        .env("GREENTIC_I18N_TRANSLATOR_BIN", &translator)
        .assert()
        .success();

    assert!(out_dir.join("assets/i18n/fr.json").is_file());
}

#[test]
fn generate_auto_translate_failure_is_nonfatal() {
    let tmp = TempDir::new().unwrap();
    let (cards_dir, out_dir, bin_dir, greentic_pack) = setup_generate(&tmp);
    let translator = support::create_failing_i18n_translator(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--greentic-pack-bin")
        .arg(&greentic_pack)
        .arg("--auto-translate")
        .arg("--langs")
        .arg("fr")
        .env("GREENTIC_I18N_TRANSLATOR_BIN", &translator)
        .assert()
        .success();

    // en.json should still be created (extraction succeeded)
    assert!(out_dir.join("assets/i18n/en.json").is_file());

    // Manifest should have translation warnings
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(out_dir.join(".cards2pack/manifest.json")).unwrap(),
    )
    .unwrap();
    let warnings = manifest.get("warnings").and_then(|v| v.as_array()).unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.get("kind").and_then(|v| v.as_str()) == Some("translation")),
        "should have translation warning"
    );
}

#[test]
fn generate_without_auto_translate_no_i18n() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    let out_dir = tmp.path().join("workspace");
    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&cards_dir).unwrap();
    fs::create_dir_all(&bin_dir).unwrap();
    write_card(&cards_dir, "card.json");
    let greentic_pack = support::create_fake_greentic_pack(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--greentic-pack-bin")
        .arg(&greentic_pack)
        .arg("--no-auto-i18n")
        .assert()
        .success();

    assert!(
        !out_dir.join("assets/i18n").exists(),
        "i18n dir should not exist with --no-auto-i18n and without --auto-translate"
    );
}
