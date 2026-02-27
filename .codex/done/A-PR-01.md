# A-PR-01 — Bootstrap `greentic-cards2pack` repo + single-command pipeline (generate workspace + dist/*.gtpack)

Date: 2026-01-20

## Goal

Create a new repo `greenticai/greentic-cards2pack` that:
- reads a directory of Adaptive Card JSON files,
- infers flows/steps/routes,
- emits an **editable pack workspace directory** (pack.yaml + flows + assets),
- and also builds `dist/<name>.gtpack` automatically in the same run by invoking `greentic-pack`.

This PR only establishes the repo structure, CLI UX, toolchain detection, workspace layout, and the end-to-end “happy path” pipeline with a **very minimal flow emitter** (stub flow file) so later PRs can evolve the flow content without reworking the orchestration.

## Non-goals (explicitly out of scope for this PR)

- Full adaptive-card parsing and route inference (done in A-PR-02/03).
- Idempotent generated-block updates inside flow files (A-PR-03).
- Graph export, strict mode, and diagnostics polish (A-PR-04).
- Any changes to `greentic-pack`, `greentic-flow`, or `greentic-dev`.

## User experience

### Command (single command only)

```bash
greentic-cards2pack generate \
  --cards ./cards \
  --out ./packs/hr-demo \
  --name hr-demo
```

Outputs:
- Workspace under `./packs/hr-demo/` with:
  - `pack.yaml`
  - `flows/` (stub for now)
  - `assets/cards/` (copies input)
  - `README.md`
  - `.cards2pack/manifest.json` (optional tracking file)
- Pack artifact:
  - `./packs/hr-demo/dist/hr-demo.gtpack`

### Flags

- `--cards <DIR>`: required
- `--out <DIR>`: required (workspace root)
- `--name <STRING>`: required (pack name & dist artifact name)
- `--greentic-pack-bin <PATH>`: optional; if absent, resolve via PATH.
- `--group-by folder|flow-field`: accepted but only stored (not yet used).
- `--default-flow <NAME>`: accepted but only stored (not yet used).
- `--strict`: accepted but only stored (not yet enforced).

## Implementation plan

### 1) Create repo scaffold

Files:
- `Cargo.toml`
- `src/main.rs`
- `src/cli.rs`
- `src/workspace.rs`
- `src/tools.rs`
- `src/errors.rs`
- `src/lib.rs` (optional, if you want integration tests to import)

Add `clap` for CLI, `anyhow` or custom error for errors, `serde_json` for manifest.

### 2) Define workspace layout

Workspace root = `--out`:
- `pack.yaml` (create if missing)
- `assets/cards/` (copy all JSON files from `--cards`, preserving subdirs)
- `flows/generated.flow.yaml` (stub for now)
- `dist/` (created if missing)
- `.cards2pack/manifest.json` (tracks inputs and config)

### 3) Implement pipeline in `generate`

Steps:
1. Validate `--cards` exists and is directory
2. Create workspace dirs
3. Copy cards into `assets/cards/` (preserve relative paths)
4. Write/ensure `pack.yaml` exists:
   - Keep extremely minimal: pack name, version, and pointers to flows/assets if required by your pack format
   - If you don’t know required keys yet, write a minimal placeholder and comment the TODO.
5. Write stub flow file in `flows/` (content minimal)
6. Invoke `greentic-pack build`:
   - args: `greentic-pack build --pack <workspace> --out <workspace>/dist`
   - After build, verify `<workspace>/dist/<name>.gtpack` exists. If `greentic-pack` emits different naming, detect newest `.gtpack` and rename/copy to `<name>.gtpack`.
7. Print summary: generated workspace path + gtpack path.

### 4) Tool invocation abstraction

`tools.rs`:
- `resolve_greentic_pack_bin()`:
  - if `--greentic-pack-bin` provided -> use it
  - else use PATH resolution (crate `which` or manual scan)
- `run_greentic_pack_build(bin, workspace, dist_dir)` using `std::process::Command`
  - capture stdout/stderr
  - on failure, surface actionable error with commandline and stderr tail

### 5) Minimal tests

Because this PR doesn’t yet do inference, tests focus on:
- workspace creation
- assets copy
- **tool invocation is mocked** (do not require real greentic-pack)

Use a test helper:
- In tests, set `GREENTIC_PACK_BIN` to a tiny helper binary/script created during test that writes a dummy `.gtpack` into dist.

Test cases:
- `generate_creates_workspace_and_dist`
- `generate_copies_cards_preserving_layout`
- `generate_renames_or_selects_gtpack_to_name`

## Detailed file changes

### Cargo.toml

Dependencies:
- `clap = { version = "4", features = ["derive"] }`
- `anyhow = "1"` (or `thiserror`)
- `serde = { version = "1", features = ["derive"] }`
- `serde_json = "1"`
- `walkdir = "2"`
- `which = "6"` (optional)

Dev dependencies:
- `tempfile = "3"`
- `assert_cmd = "2"` (optional)
- `predicates = "3"` (optional)

### src/main.rs

- Parse CLI
- Dispatch `generate`

### src/cli.rs

- Clap structs with `GenerateArgs`

### src/workspace.rs

- `create_workspace(out, name)`
- `copy_cards(cards_dir, out_assets_cards_dir)`
- `ensure_pack_yaml(...)`
- `write_stub_flow(...)`
- `ensure_dist_dir(...)`
- `write_manifest(...)`

### src/tools.rs

- Resolve and call greentic-pack

### src/errors.rs

- Custom error helpers, or skip if using anyhow

## Acceptance criteria

- `cargo build` succeeds.
- `greentic-cards2pack generate --cards <dir> --out <dir> --name <name>`:
  - creates workspace structure
  - copies cards into assets
  - writes pack.yaml and a stub flow
  - invokes greentic-pack build and results in `dist/<name>.gtpack`
- Tests pass without requiring a real greentic-pack installed.

## Notes / future PR hooks

- `GenerateArgs` already includes group-by/default-flow/strict fields so PR-02/03 can implement behavior without CLI breakage.
- workspace uses `.cards2pack/manifest.json` so PR-03 can implement idempotent updates and safe regeneration.

