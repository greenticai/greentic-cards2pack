# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`greentic-cards2pack` is a CLI tool that generates Greentic pack workspaces and `.gtpack` archives from Adaptive Card JSON files. It scans cards, builds a dependency graph, generates flow files (`.ygtc`), and packages everything into a deployable Greentic pack.

## Build & Development Commands

```bash
# Build
cargo build
cargo build --release

# Run tests
cargo test --workspace --all-features

# Run a single test
cargo test test_name

# Lint
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run integration tests (gtests)
greentic-integration-tester run --gtest tests/gtests/smoke --artifacts-dir artifacts/gtests --workdir .
```

## Required External Tools

The CLI depends on these tools being available in PATH:
- `greentic-flow` - Flow manipulation CLI
- `greentic-pack` - Pack building CLI
- `greentic-i18n-translator` (optional) - For `--auto-translate` feature

Install via:
```bash
cargo binstall greentic-flow greentic-pack greentic-i18n-translator greentic-integration-tester
```

## Architecture

### Pipeline Flow

```
Adaptive Cards (JSON) → scan → graph → emit_flow → greentic-pack → .gtpack
```

1. **scan.rs** - Scans card directory, parses JSON, extracts card metadata (cardId, flow, actions)
2. **ir.rs** - Intermediate representation structs (CardDoc, FlowGroup, Manifest, Warning)
3. **graph.rs** - Builds FlowGraph with nodes and routing edges from scanned cards
4. **emit_flow.rs** - Generates `.ygtc` flow files using `greentic-flow` CLI
5. **workspace.rs** - Orchestrates the full generation pipeline, calls external tools
6. **tools.rs** - Wrappers for `greentic-pack` subcommands
7. **diagnostics.rs** - Diagnostic reporting: warnings summary, workspace stats
8. **translate.rs** - Auto-translation pipeline: extracts strings, invokes `greentic-i18n-translator` (max 8 concurrent), generates per-locale bundles and `_manifest.json`
9. **qa_integration.rs** - Prompt2flow QA integration: interactive wizard, prompt limits, config builder
10. **i18n_extract/** - Module for extracting translatable strings from Adaptive Cards:
    - `extractor.rs` - Field-level extraction (text, title, placeholder, label, altText, errorMessage, etc.)
    - `report.rs` - Extraction report generation
    - Keys follow `{prefix}.{card_id}.{json_path}.{field}` pattern; supports skipping `$t()` patterns

### CLI Commands

Defined in `src/cli.rs`:
- `generate` - Main command:
  - `--cards`, `--out`, `--name` (required)
  - `--strict`, `--prompt`, `--auto-translate`, `--verbose`, `--no-auto-i18n` (flags)
  - `--group-by` (`Folder` | `FlowField`), `--default-flow`, `--greentic-pack-bin`
  - `--prompt-json`, `--prompt-limits` (requires `--prompt`)
  - `--langs` (comma-separated), `--glossary` (requires `--auto-translate`)
- `extract-i18n` - Extract translatable strings: `--input`, `--output`, `--prefix`, `--include-existing`, `--verbose`

### Key Patterns

**Card Identification:**
- Cards are identified by `cardId` from action data, `greentic.cardId` field, or filename stem
- Cards are grouped into flows by `flow` field, folder structure (`--group-by folder`), or `--default-flow`

**Flow Generation:**
- Uses `greentic-flow new` and `greentic-flow add-step` commands
- Generated sections are wrapped in `# BEGIN GENERATED (cards2pack)` / `# END GENERATED` markers
- Developer content outside markers is preserved across regenerations

**Strict Mode (`--strict`):**
- Missing targets cause errors instead of stub node creation
- Duplicate cardIds cause errors
- Invalid JSON causes errors

### WASM Component

`component-prompt2flow/` is a separate WASM component (target `wasm32-wasip2`) for prompt-based routing. Build with:
```bash
cargo build -p component-prompt2flow --target wasm32-wasip2 --release
```

## Environment Variables

- `GREENTIC_PACK_BIN` - Override path to `greentic-pack` binary
- `GREENTIC_I18N_TRANSLATOR_BIN` - Override path to `greentic-i18n-translator` binary
- `GREENTIC_COMPONENT_ADAPTIVE_CARD_MANIFEST` - Local component manifest for dev
- `GREENTIC_COMPONENT_ADAPTIVE_CARD_WASM` - Local component WASM for dev
- `GREENTIC_TRANSLATE_INTEGRATION` - Set to `1` to enable translate integration tests

## Test Fixtures

Located in `tests/fixtures/`:
- `cards/flow_emit/` - Cards with routes for flow emission tests
- `cards/folder_grouping/` - Nested folder structure for grouping tests
- `cards/filename_fallback/` - Filename-based card ID fallback tests
- `cards/translate/` - Translation feature test cards
- `prompt2flow/` - QA/prompt2flow integration fixtures
- `translate/` - Translation bundle fixtures

Integration tests (gtests) in `tests/gtests/smoke/`:
- `01_generate_pack.gtest` - Basic pack generation
- `02_generate_with_translate.gtest` - Pack generation with translation

## Output Structure

Generated workspace (`--out`):
```
pack.yaml                    # Pack manifest
flows/main.ygtc              # Generated flow
assets/cards/                # Copied card JSON files
assets/i18n/en.json          # English source bundle (if auto-i18n)
assets/i18n/{lang}.json      # Translated bundles (if --auto-translate)
assets/i18n/_manifest.json   # Available locales list (if --auto-translate)
assets/config/prompt2flow.json # Prompt2flow config (if --prompt)
dist/{name}.gtpack           # Final pack archive
.cards2pack/manifest.json    # Scan results and warnings
```
