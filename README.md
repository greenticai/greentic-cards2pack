# greentic-cards2pack

Generate Greentic pack workspaces and `.gtpack` archives from a directory of Adaptive Card JSON files.

This CLI scans cards, groups them into flows, generates flow files, and packages everything into a Greentic pack in a single command.

## Quick Start

1) Install:

```bash
cargo install cargo-binstall 
cargo binstall greentic-cards2pack
```

You also need:

```bash
cargo binstall greentic-flow greentic-pack
```

2) Run:

```bash
greentic-cards2pack generate \
  --cards ./cards \
  --out ./packs/hr-demo \
  --name hr-demo
```

## What You Get

`--out` becomes a full pack workspace:

```
./packs/hr-demo/
  pack.yaml
  flows/main.ygtc
  assets/cards/...
  dist/hr-demo.gtpack
  .cards2pack/manifest.json
```

The generated flow uses the Adaptive Card component:
`oci://ghcr.io/greenticai/components/component-adaptive-card:latest`.

## Common Warnings

- `ignored_file`: A JSON file under `--cards` is not an Adaptive Card (missing `type: "AdaptiveCard"`).
  - Safe to ignore if those JSON files are supporting data.
  - If it should be a card, fix the file so it has `type: "AdaptiveCard"`.

- `missing_target`: A card action references a step/cardId that does not exist in the flow.
  - In non-strict mode, a stub node is created.
  - In strict mode, this is an error and generation fails.
  - Fix by ensuring the target card exists or updating the action data.

## Tips

- Use `--strict` to enforce consistent metadata and required targets.
- The `.cards2pack/manifest.json` file records the scan results and warnings.


