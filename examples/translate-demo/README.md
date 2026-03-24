# Translate Demo

Auto-translate Adaptive Cards into multiple languages using `greentic-cards2pack` and `greentic-i18n-translator`.

## Prerequisites

```bash
cargo install greentic-cards2pack greentic-flow greentic-pack
cargo install greentic-i18n-translator   # for auto-translate
```

## Quick Start (one command)

```bash
greentic-cards2pack generate \
  --cards cards/ \
  --out my-pack \
  --name translate-demo \
  --auto-translate \
  --langs fr,de \
  --glossary glossary.json
```

This will:
1. Scan cards and extract translatable strings
2. Generate `assets/i18n/en.json` (English bundle)
3. Translate to French and German via `greentic-i18n-translator`
4. Build the `.gtpack` with all i18n bundles included

Output:
```
my-pack/
  assets/
    cards/        # copied card JSON files
    i18n/
      en.json     # English (source)
      fr.json     # French
      de.json     # German
  flows/main.ygtc
  dist/translate-demo.gtpack
```

## Step-by-Step (manual)

### 1. Extract strings

```bash
greentic-cards2pack extract-i18n \
  --input cards/ \
  --output i18n/en.json \
  --verbose
```

See `expected-output/en.json` for the expected result.

### 2. Translate

```bash
greentic-i18n-translator translate \
  --langs fr,de \
  --en i18n/en.json \
  --glossary glossary.json
```

### 3. Generate pack

```bash
greentic-cards2pack generate \
  --cards cards/ \
  --out my-pack \
  --name translate-demo
```

Then copy `i18n/` into `my-pack/assets/i18n/`.

## Glossary

`glossary.json` ensures brand names and technical terms stay untranslated:

```json
{
  "Greentic": "Greentic",
  "Dashboard": "Dashboard"
}
```

## Default Languages

If `--langs` is omitted with `--auto-translate`, these languages are used: `fr`, `de`, `es`, `ja`, `zh`.
