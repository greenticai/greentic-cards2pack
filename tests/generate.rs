use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn write_card(root: &Path, rel: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, "{ \"type\": \"AdaptiveCard\", \"actions\": [] }\n").unwrap();
}

fn write_text_card(root: &Path, rel: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let card = r#"{
  "type": "AdaptiveCard",
  "body": [
    { "type": "TextBlock", "text": "Welcome" },
    { "type": "Input.Text", "id": "name", "placeholder": "Your name" }
  ],
  "actions": [
    { "type": "Action.Submit", "title": "Continue" }
  ]
}
"#;
    fs::write(path, card).unwrap();
}

fn create_fake_greentic_pack(dir: &Path) -> PathBuf {
    if cfg!(windows) {
        let path = dir.join("greentic-pack.cmd");
        let contents = r#"@echo off
set CMD=%1
shift
if "%CMD%"=="" exit /b 1

if "%CMD%"=="new" (
  set OUT=
  :loopnew
  if "%~1"=="" goto donew
  if "%~1"=="--dir" (
    set OUT=%~2
    shift
    shift
    goto loopnew
  )
  shift
  goto loopnew
  :donew
  if "%OUT%"=="" exit /b 1
  if not exist "%OUT%" mkdir "%OUT%"
  echo name: demo> "%OUT%\pack.yaml"
  exit /b 0
)

if "%CMD%"=="update" (
  set OUT=
  :loopupdate
  if "%~1"=="" goto doneupdate
  if "%~1"=="--in" (
    set OUT=%~2
    shift
    shift
    goto loopupdate
  )
  shift
  goto loopupdate
  :doneupdate
  if "%OUT%"=="" exit /b 1
  if not exist "%OUT%" mkdir "%OUT%"
  echo name: demo> "%OUT%\pack.yaml"
  exit /b 0
)

if "%CMD%"=="doctor" (
  exit /b 0
)

if "%CMD%"=="build" (
  set OUT=
  :loopbuild
  if "%~1"=="" goto donebuild
  if "%~1"=="--gtpack-out" (
    set OUT=%~2
    shift
    shift
    goto loopbuild
  )
  shift
  goto loopbuild
  :donebuild
  if "%OUT%"=="" exit /b 1
  for %%I in ("%OUT%") do set OUTDIR=%%~dpI
  if not exist "%OUTDIR%" mkdir "%OUTDIR%"
  set NAME=%GT_PACK_NAME%
  if "%NAME%"=="" (
    type nul > "%OUT%"
  ) else (
    type nul > "%OUTDIR%%NAME%"
  )
  exit /b 0
)

exit /b 1
"#;
        fs::write(&path, contents).unwrap();
        path
    } else {
        let path = dir.join("greentic-pack");
        let contents = r#"#!/usr/bin/env bash
set -euo pipefail
cmd="${1:-}"
shift || true

case "$cmd" in
  new)
    out=""
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --dir)
          out="$2"
          shift 2
          ;;
        *)
          shift
          ;;
      esac
    done
    [[ -n "$out" ]] || { echo "missing --dir" >&2; exit 1; }
    mkdir -p "$out"
    printf "name: demo\n" > "$out/pack.yaml"
    ;;
  update)
    out=""
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --in)
          out="$2"
          shift 2
          ;;
        *)
          shift
          ;;
      esac
    done
    [[ -n "$out" ]] || { echo "missing --in" >&2; exit 1; }
    mkdir -p "$out"
    printf "name: demo\n" > "$out/pack.yaml"
    ;;
  doctor)
    ;;
  build)
    out=""
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --gtpack-out)
          out="$2"
          shift 2
          ;;
        *)
          shift
          ;;
      esac
    done
    [[ -n "$out" ]] || { echo "missing --out" >&2; exit 1; }
    mkdir -p "$(dirname "$out")"
    if [[ -n "${GT_PACK_NAME:-}" ]]; then
      : > "$(dirname "$out")/${GT_PACK_NAME}"
    else
      : > "$out"
    fi
    ;;
  *)
    echo "unknown command" >&2
    exit 1
    ;;
esac
"#;
        fs::write(&path, contents).unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
        }
        path
    }
}

#[test]
fn generate_creates_workspace_and_dist() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    let out_dir = tmp.path().join("workspace");
    fs::create_dir_all(&cards_dir).unwrap();
    write_card(&cards_dir, "card.json");

    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let greentic_pack = create_fake_greentic_pack(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--greentic-pack-bin")
        .arg(greentic_pack)
        .assert()
        .success();

    assert!(out_dir.join("pack.yaml").is_file());
    assert!(out_dir.join("README.md").is_file());
    assert!(out_dir.join("flows/main.ygtc").is_file());
    assert!(out_dir.join("assets/cards/card.json").is_file());
    assert!(out_dir.join("dist/demo.gtpack").is_file());
    assert!(out_dir.join(".cards2pack/manifest.json").is_file());
}

#[test]
fn generate_copies_cards_preserving_layout() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    let out_dir = tmp.path().join("workspace");
    fs::create_dir_all(&cards_dir).unwrap();
    write_card(&cards_dir, "hr/onboarding/card.json");
    write_card(&cards_dir, "sales/card.json");

    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let greentic_pack = create_fake_greentic_pack(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--greentic-pack-bin")
        .arg(greentic_pack)
        .assert()
        .success();

    assert!(
        out_dir
            .join("assets/cards/hr/onboarding/card.json")
            .is_file()
    );
    assert!(out_dir.join("assets/cards/sales/card.json").is_file());
}

#[test]
fn generate_renames_or_selects_gtpack_to_name() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    let out_dir = tmp.path().join("workspace");
    fs::create_dir_all(&cards_dir).unwrap();
    write_card(&cards_dir, "card.json");

    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let greentic_pack = create_fake_greentic_pack(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--greentic-pack-bin")
        .arg(greentic_pack)
        .env("GT_PACK_NAME", "unexpected.gtpack")
        .assert()
        .success();

    assert!(out_dir.join("dist/demo.gtpack").is_file());
    assert!(!out_dir.join("dist/unexpected.gtpack").is_file());

    let manifest_path = out_dir.join(".cards2pack/manifest.json");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(manifest_path).unwrap()).unwrap();
    let warnings = manifest
        .get("warnings")
        .and_then(|value| value.as_array())
        .unwrap();
    assert!(warnings.iter().any(|warning| {
        warning.get("kind").and_then(|value| value.as_str()) == Some("pack_output")
    }));
}

#[test]
fn generate_scaffolds_pack_i18n_and_rewrites_card_text() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    let out_dir = tmp.path().join("workspace");
    fs::create_dir_all(&cards_dir).unwrap();
    write_text_card(&cards_dir, "cards/welcome.json");

    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let greentic_pack = create_fake_greentic_pack(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--greentic-pack-bin")
        .arg(greentic_pack)
        .assert()
        .success();

    let rewritten_card =
        fs::read_to_string(out_dir.join("assets/cards/cards/welcome.json")).unwrap();
    assert!(rewritten_card.contains("{{i18n:cards.cards.welcome.body.i0.text}}"));
    assert!(rewritten_card.contains("{{i18n:cards.cards.welcome.body.i1.placeholder}}"));
    assert!(rewritten_card.contains("{{i18n:cards.cards.welcome.actions.i0.title}}"));

    let en: Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("assets/i18n/en.json")).unwrap())
            .unwrap();
    assert_eq!(
        en.get("cards.cards.welcome.body.i0.text")
            .and_then(Value::as_str),
        Some("Welcome")
    );
    assert_eq!(
        en.get("cards.cards.welcome.actions.i0.title")
            .and_then(Value::as_str),
        Some("Continue")
    );

    let locales: Value = serde_json::from_str(
        &fs::read_to_string(out_dir.join("assets/i18n/locales.json")).unwrap(),
    )
    .unwrap();
    assert!(locales.as_array().is_some());
    assert!(out_dir.join("assets/i18n/ar.json").is_file());
    assert!(out_dir.join("assets/i18n/ja.json").is_file());
    assert!(out_dir.join("tools/i18n.sh").is_file());

    #[cfg(unix)]
    {
        let mode = fs::metadata(out_dir.join("tools/i18n.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }
}

#[test]
fn generate_can_disable_auto_i18n() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    let out_dir = tmp.path().join("workspace");
    fs::create_dir_all(&cards_dir).unwrap();
    write_text_card(&cards_dir, "cards/welcome.json");

    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let greentic_pack = create_fake_greentic_pack(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--no-auto-i18n")
        .arg("--greentic-pack-bin")
        .arg(greentic_pack)
        .assert()
        .success();

    let card = fs::read_to_string(out_dir.join("assets/cards/cards/welcome.json")).unwrap();
    assert!(card.contains("\"Welcome\""));
    assert!(!card.contains("{{i18n:"));
    assert!(!out_dir.join("assets/i18n/en.json").exists());
    assert!(!out_dir.join("tools/i18n.sh").exists());
}

#[test]
fn generated_i18n_script_runs_with_fake_translator() {
    let tmp = TempDir::new().unwrap();
    let cards_dir = tmp.path().join("cards");
    let out_dir = tmp.path().join("workspace");
    fs::create_dir_all(&cards_dir).unwrap();
    write_text_card(&cards_dir, "cards/welcome.json");

    let bin_dir = tmp.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let greentic_pack = create_fake_greentic_pack(&bin_dir);

    cargo_bin_cmd!("greentic-cards2pack")
        .arg("generate")
        .arg("--cards")
        .arg(&cards_dir)
        .arg("--out")
        .arg(&out_dir)
        .arg("--name")
        .arg("demo")
        .arg("--greentic-pack-bin")
        .arg(greentic_pack)
        .assert()
        .success();

    let fake_bin = tmp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    let fake_translator = if cfg!(windows) {
        let path = fake_bin.join("fake-translator.cmd");
        fs::write(&path, "@echo off\r\nexit /b 0\r\n").unwrap();
        path
    } else {
        let path = fake_bin.join("fake-translator");
        fs::write(&path, "#!/usr/bin/env bash\nset -euo pipefail\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
        }
        path
    };

    let script = out_dir.join("tools/i18n.sh");
    if cfg!(windows) {
        // Script is bash-based and not expected to run in native Windows tests.
        return;
    }

    let status = std::process::Command::new("bash")
        .arg(script)
        .arg("validate")
        .current_dir(&out_dir)
        .env("TRANSLATOR_BIN", fake_translator)
        .status()
        .unwrap();
    assert!(status.success());
}
