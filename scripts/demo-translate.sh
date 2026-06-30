#!/usr/bin/env bash
# demo-translate.sh — End-to-end demo of cards2pack auto-translate feature
#
# Usage:
#   ./scripts/demo-translate.sh              # default: fr,de
#   ./scripts/demo-translate.sh ja,ko,es     # custom languages
#
# Prerequisites:
#   cargo binstall greentic-cards2pack greentic-flow greentic-pack greentic-i18n-translator

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

LANGS="${1:-fr,de}"
OUTPUT_DIR="${ROOT_DIR}/artifacts/translate-demo"
CARDS_DIR="${ROOT_DIR}/examples/translate-demo/cards"
GLOSSARY="${ROOT_DIR}/examples/translate-demo/glossary.json"
EXPECTED_EN="${ROOT_DIR}/examples/translate-demo/expected-output/en.json"
PACK_NAME="translate-demo"

# --- Colors -----------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
RESET='\033[0m'

pass() { echo -e "  ${GREEN}✔${RESET} $1"; }
fail() { echo -e "  ${RED}✘${RESET} $1"; FAILURES=$((FAILURES + 1)); }
info() { echo -e "${CYAN}→${RESET} $1"; }
header() { echo -e "\n${BOLD}$1${RESET}"; }

FAILURES=0

# --- 1. Check prerequisites ------------------------------------------
header "1. Checking prerequisites"

for bin in greentic-cards2pack greentic-flow greentic-pack greentic-i18n-translator; do
    if command -v "$bin" &>/dev/null; then
        pass "$bin found at $(command -v "$bin")"
    else
        fail "$bin not found — install with: cargo binstall $bin"
    fi
done

if [[ $FAILURES -gt 0 ]]; then
    echo -e "\n${RED}Missing prerequisites. Aborting.${RESET}"
    exit 1
fi

# --- 2. Clean previous output -----------------------------------------
header "2. Preparing output directory"

rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"
pass "Clean output at $OUTPUT_DIR"

# --- 3. Run extract-i18n (standalone) ---------------------------------
header "3. Testing extract-i18n (standalone)"

EXTRACT_OUT="$OUTPUT_DIR/extract-only/en.json"
mkdir -p "$(dirname "$EXTRACT_OUT")"

greentic-cards2pack extract-i18n \
    --input "$CARDS_DIR" \
    --output "$EXTRACT_OUT" \
    --verbose

if [[ -f "$EXTRACT_OUT" ]]; then
    EXTRACTED_KEYS=$(python3 -c "import json; print(len(json.load(open('$EXTRACT_OUT'))))")
    pass "Extracted $EXTRACTED_KEYS strings to $EXTRACT_OUT"
else
    fail "en.json not created"
fi

# Compare with expected output
if [[ -f "$EXPECTED_EN" ]]; then
    EXPECTED_KEYS=$(python3 -c "
import json
actual = set(json.load(open('$EXTRACT_OUT')).keys())
expected = set(json.load(open('$EXPECTED_EN')).keys())
missing = expected - actual
extra = actual - expected
if missing: print(f'Missing keys: {missing}')
if extra: print(f'Extra keys: {extra}')
if not missing and not extra: print('MATCH')
")
    if [[ "$EXPECTED_KEYS" == "MATCH" ]]; then
        pass "Extracted keys match expected output"
    else
        fail "Key mismatch: $EXPECTED_KEYS"
    fi
fi

# --- 4. Run full generate --auto-translate ----------------------------
header "4. Running generate --auto-translate --langs $LANGS"

greentic-cards2pack generate \
    --cards "$CARDS_DIR" \
    --out "$OUTPUT_DIR/pack" \
    --name "$PACK_NAME" \
    --auto-translate \
    --langs "$LANGS" \
    --glossary "$GLOSSARY" \
    --verbose || true  # allow greentic-pack resolve warnings

# --- 5. Verify outputs ------------------------------------------------
header "5. Verifying outputs"

# 6a. English source bundle
EN_BUNDLE="$OUTPUT_DIR/pack/assets/i18n/en.json"
if [[ -f "$EN_BUNDLE" ]]; then
    EN_KEYS=$(python3 -c "import json; print(len(json.load(open('$EN_BUNDLE'))))")
    pass "en.json exists ($EN_KEYS strings)"
else
    fail "en.json missing"
fi

# 6b. Translated bundles
IFS=',' read -ra LANG_ARRAY <<< "$LANGS"
for lang in "${LANG_ARRAY[@]}"; do
    LANG_BUNDLE="$OUTPUT_DIR/pack/assets/i18n/${lang}.json"
    if [[ -f "$LANG_BUNDLE" ]]; then
        LANG_KEYS=$(python3 -c "import json; print(len(json.load(open('$LANG_BUNDLE'))))")
        pass "${lang}.json exists ($LANG_KEYS strings)"

        # Verify all keys present
        COVERAGE=$(python3 -c "
import json
en = set(json.load(open('$EN_BUNDLE')).keys())
tr = set(json.load(open('$LANG_BUNDLE')).keys())
missing = en - tr
if missing: print(f'missing {len(missing)} keys: {missing}')
else: print('FULL')
")
        if [[ "$COVERAGE" == "FULL" ]]; then
            pass "${lang}.json has full key coverage"
        else
            fail "${lang}.json $COVERAGE"
        fi
    else
        fail "${lang}.json missing"
    fi
done

# 6c. Manifest
MANIFEST="$OUTPUT_DIR/pack/assets/i18n/_manifest.json"
if [[ -f "$MANIFEST" ]]; then
    LOCALE_COUNT=$(python3 -c "import json; print(len(json.load(open('$MANIFEST'))))")
    pass "_manifest.json exists ($LOCALE_COUNT locales)"
else
    fail "_manifest.json missing"
fi

# 6d. Flow file
FLOW_FILE="$OUTPUT_DIR/pack/flows/main.ygtc"
if [[ -f "$FLOW_FILE" ]]; then
    pass "Flow file generated"
else
    fail "Flow file missing"
fi

# 6e. Pack archive
GTPACK="$OUTPUT_DIR/pack/dist/${PACK_NAME}.gtpack"
if [[ -f "$GTPACK" ]]; then
    PACK_SIZE=$(du -h "$GTPACK" | cut -f1)
    pass "Pack archive created (${PACK_SIZE})"
else
    fail "Pack archive missing"
fi

# 6f. Manifest JSON
SCAN_MANIFEST="$OUTPUT_DIR/pack/.cards2pack/manifest.json"
if [[ -f "$SCAN_MANIFEST" ]]; then
    CARD_COUNT=$(python3 -c "import json; m=json.load(open('$SCAN_MANIFEST')); print(len(m.get('cards', [])))")
    pass "Scan manifest: $CARD_COUNT cards processed"
else
    fail "Scan manifest missing"
fi

# --- 6. Print sample translations ------------------------------------
header "6. Sample translations"

for lang in "${LANG_ARRAY[@]}"; do
    LANG_BUNDLE="$OUTPUT_DIR/pack/assets/i18n/${lang}.json"
    if [[ -f "$LANG_BUNDLE" ]]; then
        echo -e "\n  ${YELLOW}[$lang]${RESET}"
        python3 -c "
import json
data = json.load(open('$LANG_BUNDLE'))
en = json.load(open('$EN_BUNDLE'))
for key in sorted(data.keys())[:5]:
    print(f'    {key}:')
    print(f'      en: {en.get(key, \"?\")}')
    print(f'      $lang: {data[key]}')
"
    fi
done

# --- 7. Summary -------------------------------------------------------
header "7. Summary"

echo -e "  Cards:      $(ls "$CARDS_DIR"/*.json | wc -l | tr -d ' ')"
echo -e "  Strings:    $EN_KEYS"
echo -e "  Languages:  ${LANGS}"
echo -e "  Output:     $OUTPUT_DIR/pack/"
echo -e "  Pack:       $GTPACK"

if [[ $FAILURES -eq 0 ]]; then
    echo -e "\n${GREEN}${BOLD}All checks passed!${RESET}\n"
    exit 0
else
    echo -e "\n${RED}${BOLD}$FAILURES check(s) failed.${RESET}\n"
    exit 1
fi
