use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use walkdir::WalkDir;

use crate::cli::GenerateArgs;
use crate::diagnostics::{build_diagnostics, summarize, warning};
use crate::emit_flow::emit_flow;
use crate::graph::build_flow_graph;
use crate::ir::{FlowSummary, Manifest, Warning, WarningKind};
use crate::qa_integration::{
    PromptLimits, Source, build_prompt2flow_config, persist_prompt2flow_config,
    prompt_limits_from_arg,
};
use crate::scan::{ScanConfig, scan_cards};
use crate::tools::{
    resolve_greentic_pack_bin, run_greentic_pack_build, run_greentic_pack_components,
    run_greentic_pack_doctor, run_greentic_pack_new, run_greentic_pack_resolve,
    run_greentic_pack_update,
};
use serde_yaml_bw::{self, Value as YamlValue};

const COMPONENT_REF: &str = "oci://ghcr.io/greentic-ai/components/component-adaptive-card:latest";
const COMPONENT_MANIFEST_ENV: &str = "GREENTIC_COMPONENT_ADAPTIVE_CARD_MANIFEST";
const COMPONENT_WASM_ENV: &str = "GREENTIC_COMPONENT_ADAPTIVE_CARD_WASM";
const PROMPT_COMPONENT_REF: &str =
    "oci://ghcr.io/greentic-ai/components/component-prompt2flow:latest";
const PACK_I18N_LOCALES: &[&str] = &[
    "ar", "ar-AE", "ar-DZ", "ar-EG", "ar-IQ", "ar-MA", "ar-SA", "ar-SD", "ar-SY", "ar-TN", "ay",
    "bg", "bn", "cs", "da", "de", "el", "en", "en-GB", "es", "et", "fa", "fi", "fr", "gn", "gu",
    "hi", "hr", "ht", "hu", "id", "it", "ja", "km", "kn", "ko", "lo", "lt", "lv", "ml", "mr", "ms",
    "my", "nah", "ne", "nl", "no", "pa", "pl", "pt", "qu", "ro", "ru", "si", "sk", "sr", "sv",
    "ta", "te", "th", "tl", "tr", "uk", "ur", "vi", "zh",
];
const CARD_I18N_PREFIX: &str = "cards";

pub fn generate(args: &GenerateArgs) -> Result<()> {
    if !args.cards.is_dir() {
        bail!("cards directory does not exist: {}", args.cards.display());
    }

    if args.prompt_json.is_some() && !args.prompt {
        bail!("--prompt-json requires --prompt");
    }
    if args.prompt_limits.is_some() && !args.prompt {
        bail!("--prompt-limits requires --prompt");
    }

    let greentic_pack_bin = resolve_greentic_pack_bin(args.greentic_pack_bin.as_deref())?;
    let pack_yaml = args.out.join("pack.yaml");
    if !pack_yaml.exists() {
        run_greentic_pack_new(&greentic_pack_bin, &args.out, &args.name)?;
    }
    let default_flow_path = default_flow_file(&pack_yaml)?;

    fs::create_dir_all(&args.out)
        .with_context(|| format!("failed to create workspace {}", args.out.display()))?;

    let assets_cards = args.out.join("assets").join("cards");
    let flows_dir = args.out.join("flows");
    let dist_dir = args.out.join("dist");
    let state_dir = args.out.join(".cards2pack");

    fs::create_dir_all(&assets_cards)
        .with_context(|| format!("failed to create {}", assets_cards.display()))?;
    fs::create_dir_all(&flows_dir)
        .with_context(|| format!("failed to create {}", flows_dir.display()))?;
    fs::create_dir_all(&dist_dir)
        .with_context(|| format!("failed to create {}", dist_dir.display()))?;
    fs::create_dir_all(&state_dir)
        .with_context(|| format!("failed to create {}", state_dir.display()))?;

    copy_cards(&args.cards, &assets_cards)?;
    if !args.no_auto_i18n {
        scaffold_pack_i18n(&args.out, &assets_cards)?;
    }
    ensure_readme(&args.out, &args.name)?;

    let prompt_limits = if args.prompt {
        prompt_limits_from_arg(args.prompt_limits.as_deref())?.unwrap_or_default()
    } else {
        PromptLimits::default()
    };

    if args.prompt {
        let source = args
            .prompt_json
            .as_deref()
            .map(Source::JsonFile)
            .unwrap_or(Source::Interactive);
        let config = build_prompt2flow_config(source, prompt_limits)?;
        let prompt_config_path = args
            .out
            .join("assets")
            .join("config")
            .join("prompt2flow.json");
        persist_prompt2flow_config(&config, &prompt_config_path)?;
    }

    let scan_config = ScanConfig {
        cards_dir: assets_cards.clone(),
        group_by: args.group_by,
        default_flow: args.default_flow.clone(),
        strict: args.strict,
    };
    let mut manifest = scan_cards(&scan_config)?;

    let mut flow_paths = Vec::new();
    let mut readme_entries = Vec::new();
    for flow in &manifest.flows {
        let graph = build_flow_graph(flow, args.strict)?;
        if !graph.warnings.is_empty() {
            manifest.warnings.extend(graph.warnings.iter().cloned());
        }
        let (path, flow_warnings) = emit_flow(&graph, &args.out, args.strict)?;
        if !flow_warnings.is_empty() {
            manifest.warnings.extend(flow_warnings);
        }
        let is_prompt_flow = args.prompt
            && default_flow_path
                .as_ref()
                .map(|default| default == &path)
                .unwrap_or(false);
        if is_prompt_flow {
            insert_prompt_node(&path)?;
        }
        write_flow_resolve_sidecar(&path, &graph)?;
        if is_prompt_flow {
            extend_sidecar_with_prompt(&path)?;
        }
        let flow_path = path
            .strip_prefix(&args.out)
            .unwrap_or(&path)
            .display()
            .to_string();
        if !flow_paths.contains(&flow_path) {
            flow_paths.push(flow_path);
        }
        let entry = graph
            .nodes
            .values()
            .find(|node| !node.stub)
            .map(|node| node.name.clone())
            .unwrap_or_else(|| "unknown".to_string());
        readme_entries.push((flow.flow_name.clone(), entry));
    }

    sync_local_component_if_configured(&args.out, &greentic_pack_bin, &mut manifest, args.strict)?;
    run_greentic_pack_update(&greentic_pack_bin, &args.out)?;
    update_readme(&args.out, &args.name, &readme_entries)?;

    if let Err(err) = run_greentic_flow_doctor(&args.out.join("flows")) {
        if args.strict {
            return Err(err);
        }
        manifest.warnings.push(warning(
            WarningKind::Validation,
            format!("greentic-flow doctor failed: {err}"),
        ));
    }

    if let Err(err) = run_greentic_pack_resolve(&greentic_pack_bin, &args.out) {
        if args.strict {
            return Err(err);
        }
        manifest.warnings.push(warning(
            WarningKind::Validation,
            format!("greentic-pack resolve failed: {err}"),
        ));
    }

    if let Err(err) = run_greentic_pack_doctor(&greentic_pack_bin, &args.out) {
        if args.strict {
            return Err(err);
        }
        manifest.warnings.push(warning(
            WarningKind::Validation,
            format!("greentic-pack doctor failed: {err}"),
        ));
    }

    let gtpack_out = dist_dir.join(format!("{}.gtpack", args.name));
    let build_output =
        run_greentic_pack_build(&greentic_pack_bin, &args.out, &gtpack_out, args.verbose)?;
    if !gtpack_out.exists()
        && let Some(path) = extract_gtpack_path(&build_output)
        && path.exists()
    {
        fs::copy(&path, &gtpack_out).with_context(|| {
            format!(
                "failed to copy greentic-pack output {} to {}",
                path.display(),
                gtpack_out.display()
            )
        })?;
    }

    let (gtpack_path, gtpack_warning) = ensure_named_gtpack(&dist_dir, &args.name)?;
    if let Some(warning) = gtpack_warning {
        manifest.warnings.push(warning);
    }

    let flow_summaries: Vec<FlowSummary> = manifest
        .flows
        .iter()
        .map(|flow| FlowSummary {
            flow_name: flow.flow_name.clone(),
            card_count: flow.cards.len(),
        })
        .collect();
    manifest.diagnostics = build_diagnostics(
        args.out.clone(),
        Some(gtpack_path.clone()),
        flow_paths.clone(),
        flow_summaries,
        manifest.flows.iter().map(|flow| flow.cards.len()).sum(),
        manifest.warnings.len(),
    );
    write_manifest(&state_dir, &manifest)?;

    println!("{}", summarize(&manifest.diagnostics, &manifest.warnings));

    Ok(())
}

fn scaffold_pack_i18n(workspace_root: &Path, cards_root: &Path) -> Result<()> {
    let i18n_dir = workspace_root.join("assets").join("i18n");
    fs::create_dir_all(&i18n_dir)
        .with_context(|| format!("failed to create {}", i18n_dir.display()))?;

    let mut extracted = BTreeMap::<String, String>::new();
    rewrite_cards_with_i18n_markers(cards_root, &mut extracted)?;

    let en_path = i18n_dir.join("en.json");
    write_en_locale(&en_path, extracted)?;

    let locales_path = i18n_dir.join("locales.json");
    let locales_json =
        serde_json::to_string_pretty(PACK_I18N_LOCALES).context("serialize locales.json")?;
    fs::write(&locales_path, format!("{locales_json}\n"))
        .with_context(|| format!("failed to write {}", locales_path.display()))?;

    ensure_locale_files(&i18n_dir)?;
    write_pack_i18n_script(workspace_root)?;
    Ok(())
}

fn rewrite_cards_with_i18n_markers(
    cards_root: &Path,
    extracted: &mut BTreeMap<String, String>,
) -> Result<()> {
    for entry in WalkDir::new(cards_root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let raw = match fs::read_to_string(path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let mut value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let is_adaptive_card =
            value.get("type").and_then(serde_json::Value::as_str) == Some("AdaptiveCard");
        if !is_adaptive_card {
            continue;
        }

        let rel = path
            .strip_prefix(cards_root)
            .with_context(|| format!("failed to strip prefix for {}", path.display()))?;
        let card_prefix = card_key_prefix(rel);
        let mut key_path = vec![CARD_I18N_PREFIX.to_string()];
        key_path.extend(card_prefix);

        let mut changed = false;
        rewrite_value_for_i18n(&mut value, &mut key_path, extracted, &mut changed);
        if changed {
            let encoded = serde_json::to_string_pretty(&value).context("encode adaptive card")?;
            fs::write(path, format!("{encoded}\n"))
                .with_context(|| format!("failed to write {}", path.display()))?;
        }
    }
    Ok(())
}

fn card_key_prefix(rel_path: &Path) -> Vec<String> {
    let mut segments = Vec::new();
    for component in rel_path.components() {
        segments.push(component.as_os_str().to_string_lossy().to_string());
    }
    if let Some(last) = segments.last_mut()
        && let Some(stem) = Path::new(last).file_stem().and_then(|stem| stem.to_str())
    {
        *last = stem.to_string();
    }
    segments
        .into_iter()
        .map(|segment| sanitize_key_segment(&segment))
        .collect()
}

fn rewrite_value_for_i18n(
    value: &mut serde_json::Value,
    key_path: &mut Vec<String>,
    extracted: &mut BTreeMap<String, String>,
    changed: &mut bool,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                key_path.push(sanitize_key_segment(key));
                if should_localize_field(key, child) {
                    if let serde_json::Value::String(text) = child {
                        let i18n_key = key_path.join(".");
                        let original = text.clone();
                        extracted.insert(i18n_key.clone(), original);
                        let marker = format!("{{{{i18n:{i18n_key}}}}}");
                        if *text != marker {
                            *text = marker;
                            *changed = true;
                        }
                    }
                } else {
                    rewrite_value_for_i18n(child, key_path, extracted, changed);
                }
                key_path.pop();
            }
        }
        serde_json::Value::Array(items) => {
            for (idx, item) in items.iter_mut().enumerate() {
                key_path.push(format!("i{idx}"));
                rewrite_value_for_i18n(item, key_path, extracted, changed);
                key_path.pop();
            }
        }
        _ => {}
    }
}

fn should_localize_field(key: &str, value: &serde_json::Value) -> bool {
    let is_text_field = matches!(
        key,
        "text" | "title" | "placeholder" | "errorMessage" | "label" | "altText" | "speak"
    );
    if !is_text_field {
        return false;
    }
    let Some(text) = value.as_str() else {
        return false;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains("{{") && !trimmed.contains("{{i18n:") {
        return false;
    }
    if trimmed.starts_with("{{i18n:") && trimmed.ends_with("}}") {
        return false;
    }
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("mailto:")
    {
        return false;
    }
    true
}

fn sanitize_key_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let normalized = out.trim_matches('_');
    if normalized.is_empty() {
        "field".to_string()
    } else {
        normalized.to_string()
    }
}

fn write_en_locale(en_path: &Path, extracted: BTreeMap<String, String>) -> Result<()> {
    let mut merged = BTreeMap::<String, String>::new();
    if en_path.exists()
        && let Ok(raw) = fs::read_to_string(en_path)
        && let Ok(existing) = serde_json::from_str::<BTreeMap<String, String>>(&raw)
    {
        for (key, value) in existing {
            if !key.starts_with(&format!("{CARD_I18N_PREFIX}.")) {
                merged.insert(key, value);
            }
        }
    }
    for (key, value) in extracted {
        merged.insert(key, value);
    }
    let encoded = serde_json::to_string_pretty(&merged).context("serialize en.json")?;
    fs::write(en_path, format!("{encoded}\n"))
        .with_context(|| format!("failed to write {}", en_path.display()))?;
    Ok(())
}

fn ensure_locale_files(i18n_dir: &Path) -> Result<()> {
    for locale in PACK_I18N_LOCALES {
        if *locale == "en" {
            continue;
        }
        let locale_path = i18n_dir.join(format!("{locale}.json"));
        if locale_path.exists() {
            continue;
        }
        fs::write(&locale_path, "{\n}\n")
            .with_context(|| format!("failed to write {}", locale_path.display()))?;
    }
    Ok(())
}

fn write_pack_i18n_script(workspace_root: &Path) -> Result<()> {
    let tools_dir = workspace_root.join("tools");
    fs::create_dir_all(&tools_dir)
        .with_context(|| format!("failed to create {}", tools_dir.display()))?;
    let script_path = tools_dir.join("i18n.sh");
    fs::write(&script_path, PACK_I18N_SCRIPT)
        .with_context(|| format!("failed to write {}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms)?;
    }
    Ok(())
}

const PACK_I18N_SCRIPT: &str = r#"#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

I18N_DIR="$ROOT_DIR/assets/i18n"
LOCALES_FILE="$I18N_DIR/locales.json"
EN_FILE="$I18N_DIR/en.json"
MODE="${1:-all}"
LOCALE="${LOCALE:-en}"
AUTH_MODE="${AUTH_MODE:-auto}"
TRANSLATOR_BIN="${TRANSLATOR_BIN:-greentic-i18n-translator}"
CODEX_SKIP_GIT_REPO_CHECK="${CODEX_SKIP_GIT_REPO_CHECK:-1}"

fail() {
  echo "error: $*" >&2
  exit 1
}

info() {
  echo "info: $*"
}

usage() {
  cat <<'USAGE'
Usage: tools/i18n.sh [all|translate|validate|status]

Environment overrides:
  LOCALE=...          Locale for translator runtime messages (default: en)
  AUTH_MODE=...       Auth mode for translate (default: auto)
  TRANSLATOR_BIN=...  Translator command (default: greentic-i18n-translator)
  CODEX_SKIP_GIT_REPO_CHECK=0|1  Add --skip-git-repo-check to codex exec (default: 1)
USAGE
}

require_files() {
  [[ -f "$LOCALES_FILE" ]] || fail "missing $LOCALES_FILE"
  [[ -f "$EN_FILE" ]] || fail "missing $EN_FILE"
}

resolve_translator_bin() {
  if command -v "$TRANSLATOR_BIN" >/dev/null 2>&1; then
    return 0
  fi
  fail "translator binary not found on PATH: $TRANSLATOR_BIN"
}

load_locales() {
  mapfile -t LOCALES < <(python3 - <<'PY' "$LOCALES_FILE"
import json, sys
for item in json.load(open(sys.argv[1], encoding="utf-8")):
    print(item)
PY
)
  LOCALES_CSV="$(IFS=,; echo "${LOCALES[*]}")"
}

ensure_locale_files() {
  for locale in "${LOCALES[@]}"; do
    file="$I18N_DIR/$locale.json"
    if [[ ! -f "$file" ]]; then
      mkdir -p "$(dirname "$file")"
      printf "{\n}\n" > "$file"
    fi
  done
}

setup_codex_wrapper_if_needed() {
  if [[ "$CODEX_SKIP_GIT_REPO_CHECK" != "1" ]]; then
    return 0
  fi
  if ! command -v codex >/dev/null 2>&1; then
    return 0
  fi
  local real_codex
  real_codex="$(command -v codex)"
  local wrapper_dir
  wrapper_dir="$(mktemp -d)"
  cat > "$wrapper_dir/codex" <<EOF
#!/usr/bin/env bash
set -euo pipefail
if [[ "\${1:-}" == "exec" ]]; then
  shift
  exec "$real_codex" exec --skip-git-repo-check "\$@"
fi
exec "$real_codex" "\$@"
EOF
  chmod +x "$wrapper_dir/codex"
  export PATH="$wrapper_dir:$PATH"
}

run_translate() {
  info "running translate for $(echo "$LOCALES_CSV" | tr ',' ' ' | wc -w) locales from $EN_FILE"
  setup_codex_wrapper_if_needed
  "$TRANSLATOR_BIN" --locale "$LOCALE" \
    translate --langs "$LOCALES_CSV" --en "$EN_FILE" --auth-mode "$AUTH_MODE"
}

run_validate() {
  info "running validate for $(echo "$LOCALES_CSV" | tr ',' ' ' | wc -w) locales from $EN_FILE"
  "$TRANSLATOR_BIN" --locale "$LOCALE" \
    validate --langs "$LOCALES_CSV" --en "$EN_FILE"
}

run_status() {
  info "running status for $(echo "$LOCALES_CSV" | tr ',' ' ' | wc -w) locales from $EN_FILE"
  "$TRANSLATOR_BIN" --locale "$LOCALE" \
    status --langs "$LOCALES_CSV" --en "$EN_FILE"
}

if [[ "$MODE" == "-h" || "$MODE" == "--help" ]]; then
  usage
  exit 0
fi

require_files
resolve_translator_bin
load_locales
ensure_locale_files
info "mode=$MODE locale=$LOCALE translator=$TRANSLATOR_BIN"

case "$MODE" in
  all)
    run_translate
    run_validate
    run_status
    ;;
  translate)
    run_translate
    ;;
  validate)
    run_validate
    ;;
  status)
    run_status
    ;;
  *)
    usage
    exit 2
    ;;
esac
"#;

fn copy_cards(cards_dir: &Path, dest_root: &Path) -> Result<()> {
    for entry in WalkDir::new(cards_dir).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let extension = path.extension().and_then(|ext| ext.to_str());
        if extension.is_none_or(|ext| !ext.eq_ignore_ascii_case("json")) {
            continue;
        }

        let rel = path
            .strip_prefix(cards_dir)
            .with_context(|| format!("failed to strip prefix for {}", path.display()))?;
        let dest_path = dest_root.join(rel);
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(path, &dest_path).with_context(|| format!("failed to copy {}", path.display()))?;
    }

    Ok(())
}

fn default_flow_file(pack_yaml: &Path) -> Result<Option<PathBuf>> {
    let contents =
        fs::read_to_string(pack_yaml).with_context(|| format!("read {}", pack_yaml.display()))?;
    let manifest: YamlValue =
        serde_yaml_bw::from_str(&contents).context("parse pack manifest yaml for default flow")?;
    let flows = manifest
        .get("flows")
        .and_then(YamlValue::as_sequence)
        .cloned()
        .unwrap_or_default();
    let root = pack_yaml.parent().unwrap_or_else(|| Path::new("."));
    for candidate in &flows {
        if let Some(entrypoints) = candidate
            .get("entrypoints")
            .and_then(YamlValue::as_sequence)
            && entrypoints
                .iter()
                .any(|entry| entry.as_str() == Some("default"))
            && let Some(file) = candidate.get("file").and_then(YamlValue::as_str)
        {
            return Ok(Some(root.join(file)));
        }
    }
    if let Some(first) = flows.first()
        && let Some(file) = first.get("file").and_then(YamlValue::as_str)
    {
        return Ok(Some(root.join(file)));
    }
    Ok(None)
}

fn insert_prompt_node(flow_path: &Path) -> Result<()> {
    let contents =
        fs::read_to_string(flow_path).with_context(|| format!("read {}", flow_path.display()))?;
    let nodes = extract_node_order(&contents, flow_path)?;

    if nodes
        .first()
        .map(|name| name == "prompt2flow")
        .unwrap_or(false)
    {
        return Ok(());
    }
    if let Some(index) = nodes.iter().position(|name| name == "prompt2flow") {
        let node_name = &nodes[index];
        bail!(
            "prompt2flow node '{}' exists in {} but is not the first node (index={}): move it to the start or regenerate with --prompt",
            node_name,
            flow_path.display(),
            index
        );
    }

    let first_node_name = nodes.first().expect("nodes should be present");
    let marker = "nodes:\n";
    let insert_pos = contents
        .find(marker)
        .map(|idx| idx + marker.len())
        .ok_or_else(|| anyhow!("flow {} missing nodes section", flow_path.display()))?;
    let snippet = format!(
        "  prompt2flow:\n    routing:\n    - to: {first}\n    component.exec:\n      component: ai.greentic.component-prompt2flow\n      operation: handle_message\n      input:\n        config_path: assets/config/prompt2flow.json\n\n",
        first = first_node_name
    );
    let new_contents = format!(
        "{}{}{}",
        &contents[..insert_pos],
        snippet,
        &contents[insert_pos..]
    );
    fs::write(flow_path, new_contents)
        .with_context(|| format!("write modified flow {}", flow_path.display()))?;
    Ok(())
}

fn extract_node_order(contents: &str, flow_path: &Path) -> Result<Vec<String>> {
    let marker = "nodes:\n";
    let start = contents
        .find(marker)
        .map(|idx| idx + marker.len())
        .ok_or_else(|| anyhow!("flow {} missing nodes section", flow_path.display()))?;
    let mut nodes = Vec::new();
    for line in contents[start..].lines() {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.chars().take_while(|c| *c == ' ').count();
        if indent < 2 {
            break;
        }
        if indent != 2 {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('-') {
            continue;
        }
        if let Some(name) = trimmed.strip_suffix(':') {
            nodes.push(name.to_string());
            continue;
        }
        break;
    }
    if nodes.is_empty() {
        bail!("flow {} has no nodes", flow_path.display());
    }
    Ok(nodes)
}

fn extend_sidecar_with_prompt(flow_path: &Path) -> Result<()> {
    let sidecar_path = flow_path.with_extension("ygtc.resolve.json");
    let mut payload: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&sidecar_path)
            .with_context(|| format!("read {}", sidecar_path.display()))?,
    )
    .context("parse flow resolve sidecar")?;
    let nodes = payload
        .get_mut("nodes")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| anyhow!("missing nodes map in {}", sidecar_path.display()))?;
    if nodes.contains_key("prompt2flow") {
        return Ok(());
    }
    nodes.insert(
        "prompt2flow".to_string(),
        serde_json::json!({
            "source": {
                "kind": "oci",
                "ref": PROMPT_COMPONENT_REF
            }
        }),
    );
    fs::write(
        &sidecar_path,
        serde_json::to_string_pretty(&payload).context("serialize updated sidecar")?,
    )
    .with_context(|| format!("write {}", sidecar_path.display()))?;
    Ok(())
}

fn sync_local_component_if_configured(
    pack_root: &Path,
    greentic_pack_bin: &Path,
    manifest: &mut Manifest,
    strict: bool,
) -> Result<()> {
    let manifest_path = match env::var(COMPONENT_MANIFEST_ENV) {
        Ok(value) if !value.trim().is_empty() => Some(PathBuf::from(value)),
        _ => None,
    };
    let wasm_path = match env::var(COMPONENT_WASM_ENV) {
        Ok(value) if !value.trim().is_empty() => Some(PathBuf::from(value)),
        _ => None,
    };

    if manifest_path.is_none() && wasm_path.is_none() {
        return Ok(());
    }

    let manifest_path = match manifest_path {
        Some(path) => path,
        None => {
            let message = format!(
                "{} is set but {} is not",
                COMPONENT_WASM_ENV, COMPONENT_MANIFEST_ENV
            );
            if strict {
                return Err(anyhow!(message));
            }
            manifest
                .warnings
                .push(warning(WarningKind::PackOutput, message));
            return Ok(());
        }
    };
    let wasm_path = match wasm_path {
        Some(path) => path,
        None => {
            let message = format!(
                "{} is set but {} is not",
                COMPONENT_MANIFEST_ENV, COMPONENT_WASM_ENV
            );
            if strict {
                return Err(anyhow!(message));
            }
            manifest
                .warnings
                .push(warning(WarningKind::PackOutput, message));
            return Ok(());
        }
    };

    let pack_version = pack_yaml_version(pack_root);
    let manifest_contents = fs::read_to_string(&manifest_path).with_context(|| {
        format!(
            "failed to read component manifest {}",
            manifest_path.display()
        )
    })?;
    let mut manifest_json: serde_json::Value = serde_json::from_str(&manifest_contents)
        .with_context(|| format!("invalid component manifest {}", manifest_path.display()))?;
    if let Some(version) = pack_version
        && manifest_json
            .get("version")
            .and_then(|value| value.as_str())
            != Some(version.as_str())
    {
        manifest_json["version"] = serde_json::Value::String(version);
    }
    let wasm_name = manifest_json
        .pointer("/artifacts/component_wasm")
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| {
            wasm_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("component_adaptive_card.wasm")
        });

    let components_dir = pack_root.join("components").join("component-adaptive-card");
    fs::create_dir_all(&components_dir)
        .with_context(|| format!("failed to create {}", components_dir.display()))?;
    fs::write(
        components_dir.join("component.manifest.json"),
        serde_json::to_string_pretty(&manifest_json)?,
    )
    .with_context(|| {
        format!(
            "failed to write component manifest to {}",
            components_dir.display()
        )
    })?;
    fs::copy(&wasm_path, components_dir.join(wasm_name))
        .with_context(|| format!("failed to copy component wasm from {}", wasm_path.display()))?;

    run_greentic_pack_components(greentic_pack_bin, pack_root)?;
    Ok(())
}

fn pack_yaml_version(pack_root: &Path) -> Option<String> {
    let contents = fs::read_to_string(pack_root.join("pack.yaml")).ok()?;
    let yaml: serde_yaml_bw::Value = serde_yaml_bw::from_str(&contents).ok()?;
    yaml.get("version")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn ensure_readme(workspace: &Path, name: &str) -> Result<()> {
    let readme_path = workspace.join("README.md");
    if readme_path.exists() {
        return Ok(());
    }

    let contents = format!(
        "# {name}\n\nGenerated by greentic-cards2pack.\n",
        name = name
    );

    fs::write(&readme_path, contents)
        .with_context(|| format!("failed to write {}", readme_path.display()))?;

    Ok(())
}

fn write_manifest(state_dir: &Path, manifest: &Manifest) -> Result<()> {
    let path = state_dir.join("manifest.json");
    let json = serde_json::to_vec_pretty(&manifest)?;
    let mut file =
        fs::File::create(&path).with_context(|| format!("failed to write {}", path.display()))?;
    file.write_all(&json)?;
    file.write_all(b"\n")?;

    Ok(())
}

fn update_readme(workspace: &Path, name: &str, entries: &[(String, String)]) -> Result<()> {
    let readme_path = workspace.join("README.md");
    let existing = if readme_path.exists() {
        fs::read_to_string(&readme_path)
            .with_context(|| format!("failed to read {}", readme_path.display()))?
    } else {
        format!("# {name}\n\nGenerated by greentic-cards2pack.\n")
    };

    let mut section = String::new();
    section.push_str("<!-- BEGIN GENERATED FLOWS (cards2pack) -->\n");
    section.push_str("## Generated Flows\n");
    if entries.is_empty() {
        section.push_str("- (none)\n");
    } else {
        for (flow, entry) in entries {
            section.push_str(&format!("- `{flow}` entry: `{entry}`\n"));
        }
    }
    section.push_str("<!-- END GENERATED FLOWS (cards2pack) -->\n");

    let updated = replace_marked_section(
        &existing,
        "<!-- BEGIN GENERATED FLOWS (cards2pack) -->",
        "<!-- END GENERATED FLOWS (cards2pack) -->",
        &section,
    );

    fs::write(&readme_path, updated)
        .with_context(|| format!("failed to write {}", readme_path.display()))?;

    Ok(())
}

fn run_greentic_flow_doctor(flows_dir: &Path) -> Result<()> {
    if !flows_dir.is_dir() {
        return Ok(());
    }

    let status = std::process::Command::new("greentic-flow")
        .arg("doctor")
        .arg(flows_dir)
        .status()
        .with_context(|| {
            format!(
                "failed to run greentic-flow doctor for {}",
                flows_dir.display()
            )
        })?;

    if !status.success() {
        bail!("greentic-flow doctor failed for {}", flows_dir.display());
    }

    Ok(())
}

fn replace_marked_section(existing: &str, start: &str, end: &str, section: &str) -> String {
    let start_pos = existing.find(start);
    let end_pos = existing.find(end);

    match (start_pos, end_pos) {
        (Some(start_pos), Some(end_pos)) if end_pos > start_pos => {
            let after_end = existing[end_pos..].find('\n').map(|idx| end_pos + idx + 1);
            let before = &existing[..start_pos];
            let after = after_end.map_or("", |idx| &existing[idx..]);
            format!("{before}{section}{after}")
        }
        _ => {
            if existing.trim().is_empty() {
                section.to_string()
            } else {
                format!("{existing}\n{section}")
            }
        }
    }
}

fn extract_gtpack_path(build_output: &crate::tools::BuildOutput) -> Option<PathBuf> {
    for line in build_output
        .stdout
        .lines()
        .chain(build_output.stderr.lines())
    {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("wrote ") {
            let candidate = rest.trim();
            if candidate.ends_with(".gtpack") {
                return Some(PathBuf::from(candidate));
            }
        }
    }
    None
}

fn write_flow_resolve_sidecar(flow_path: &Path, graph: &crate::graph::FlowGraph) -> Result<()> {
    let component_source = if let Some(local_wasm) = component_wasm_path(flow_path) {
        serde_json::json!({
            "kind": "local",
            "path": format!("file://{local_wasm}")
        })
    } else {
        serde_json::json!({
            "kind": "oci",
            "ref": COMPONENT_REF
        })
    };

    let mut nodes = serde_json::Map::new();
    for node in graph.nodes.keys() {
        nodes.insert(
            node.clone(),
            serde_json::json!({
                "source": component_source
            }),
        );
    }

    let payload = serde_json::json!({
        "schema_version": 1,
        "flow": flow_path.file_name().and_then(|name| name.to_str()).unwrap_or("main.ygtc"),
        "nodes": nodes
    });

    let sidecar_path = flow_path.with_extension("ygtc.resolve.json");
    fs::write(&sidecar_path, serde_json::to_string_pretty(&payload)?)
        .with_context(|| format!("failed to write {}", sidecar_path.display()))?;

    let summary_path = flow_path.with_extension("ygtc.resolve.summary.json");
    if summary_path.exists() {
        let _ = fs::remove_file(&summary_path);
    }

    Ok(())
}

fn component_wasm_path(flow_path: &Path) -> Option<String> {
    let value = env::var(COMPONENT_WASM_ENV).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let flow_dir = flow_path.parent()?;
    let abs = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        env::current_dir().ok()?.join(trimmed)
    };
    let flow_dir_abs = if flow_dir.is_absolute() {
        flow_dir.to_path_buf()
    } else {
        env::current_dir().ok()?.join(flow_dir)
    };
    let rel = relative_path(&flow_dir_abs, &abs).unwrap_or(abs);
    Some(rel.to_string_lossy().to_string())
}

fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    let base_components: Vec<_> = base.components().collect();
    let target_components: Vec<_> = target.components().collect();
    let mut common = 0;
    while common < base_components.len()
        && common < target_components.len()
        && base_components[common] == target_components[common]
    {
        common += 1;
    }
    let mut rel = PathBuf::new();
    for _ in common..base_components.len() {
        rel.push("..");
    }
    for component in &target_components[common..] {
        rel.push(component.as_os_str());
    }
    if rel.as_os_str().is_empty() {
        None
    } else {
        Some(rel)
    }
}

fn ensure_named_gtpack(dist_dir: &Path, name: &str) -> Result<(PathBuf, Option<Warning>)> {
    let target_name = format!("{name}.gtpack");
    let target_path = dist_dir.join(&target_name);
    if target_path.exists() {
        return Ok((target_path, None));
    }

    let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in fs::read_dir(dist_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("gtpack") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let replace = newest
            .as_ref()
            .map(|(_, time)| modified > *time)
            .unwrap_or(true);
        if replace {
            newest = Some((path, modified));
        }
    }

    let (source, _) =
        newest.ok_or_else(|| anyhow!("no .gtpack file found in {}", dist_dir.display()))?;
    let normalized_warning = warning(
        WarningKind::PackOutput,
        format!(
            "normalized gtpack output from {} to {}",
            source.display(),
            target_path.display()
        ),
    );

    if source != target_path && fs::rename(&source, &target_path).is_err() {
        fs::copy(&source, &target_path)?;
        fs::remove_file(&source)?;
    }

    Ok((target_path, Some(normalized_warning)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const BASE_FLOW: &str = "\
nodes:
  start:
    component.exec:
      component: dummy
  follow:
    component.exec:
      component: dummy
";

    fn write_flow(contents: &str) -> (TempDir, PathBuf) {
        let tmp = TempDir::new().expect("temp dir");
        let flow = tmp.path().join("flow.ygtc");
        fs::write(&flow, contents).expect("write flow");
        (tmp, flow)
    }

    #[test]
    fn prompt_node_inserts_before_first_node() {
        let (_tmp, flow_path) = write_flow(BASE_FLOW);
        insert_prompt_node(&flow_path).expect("insert prompt node");
        let updated = fs::read_to_string(&flow_path).expect("read updated flow");
        let prompt_index = updated.find("prompt2flow:").expect("has prompt node");
        let start_index = updated.find("start:").expect("has start node");
        assert!(prompt_index < start_index);
        assert_eq!(
            updated.matches("prompt2flow:").count(),
            1,
            "should not duplicate prompt node"
        );
    }

    #[test]
    fn prompt_node_is_idempotent() {
        let flow_contents = "\
nodes:
  prompt2flow:
    routing:
    - to: start
  start:
    component.exec:
      component: dummy
";
        let (_tmp, flow_path) = write_flow(flow_contents);
        insert_prompt_node(&flow_path).expect("insert prompt node should no-op");
        let updated = fs::read_to_string(&flow_path).expect("read updated flow");
        assert_eq!(updated.matches("prompt2flow:").count(), 1);
        assert!(updated.find("prompt2flow:").unwrap() < updated.find("start:").unwrap());
    }

    #[test]
    fn prompt_node_error_when_not_first() {
        let flow_contents = "\
nodes:
  start:
    component.exec:
      component: dummy
  prompt2flow:
    routing:
    - to: start
";
        let (_tmp, flow_path) = write_flow(flow_contents);
        let err =
            insert_prompt_node(&flow_path).expect_err("should fail when prompt node not first");
        let message = err.to_string();
        assert!(message.contains("prompt2flow node 'prompt2flow' exists"));
        assert!(message.contains("index=1"));
        assert!(message.contains(flow_path.to_str().unwrap()));
    }
}
