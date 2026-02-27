use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

pub fn copy_fixture_cards(src: &Path, dest: &Path) {
    for entry in WalkDir::new(src).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(src).unwrap();
        let dest_path = dest.join(rel);
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::copy(entry.path(), dest_path).unwrap();
    }
}

pub fn create_fake_greentic_pack(dir: &Path) -> PathBuf {
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
  (
    echo name: demo> "%OUT%\pack.yaml"
    echo flows:>> "%OUT%\pack.yaml"
    echo   - file: flows/main.ygtc>> "%OUT%\pack.yaml"
    echo     entrypoints:>> "%OUT%\pack.yaml"
    echo       - default>> "%OUT%\pack.yaml"
  )
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
  (
    echo name: demo> "%OUT%\pack.yaml"
    echo flows:>> "%OUT%\pack.yaml"
    echo   - file: flows/main.ygtc>> "%OUT%\pack.yaml"
    echo     entrypoints:>> "%OUT%\pack.yaml"
    echo       - default>> "%OUT%\pack.yaml"
  )
  if exist "%OUT%\assets\config\prompt2flow.json" (
    echo components:>> "%OUT%\pack.yaml"
    echo   - id: ai.greentic.component-prompt2flow>> "%OUT%\pack.yaml"
    echo     ref: oci://ghcr.io/greenticai/components/component-prompt2flow:latest>> "%OUT%\pack.yaml"
  )
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
    cat <<'EOF' > "$out/pack.yaml"
name: demo
flows:
  - file: flows/main.ygtc
    entrypoints:
      - default
EOF
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
    cat <<'EOF' > "$out/pack.yaml"
name: demo
flows:
  - file: flows/main.ygtc
    entrypoints:
      - default
EOF
    if [[ -f "$out/assets/config/prompt2flow.json" ]]; then
      cat <<'EOF' >> "$out/pack.yaml"
components:
  - id: ai.greentic.component-prompt2flow
    ref: oci://ghcr.io/greenticai/components/component-prompt2flow:latest
EOF
    fi
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
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
        }
        path
    }
}

