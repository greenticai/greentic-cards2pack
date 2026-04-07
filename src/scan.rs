use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use walkdir::WalkDir;

use crate::cli::GroupBy;
use crate::diagnostics::warning;
use crate::ir::{
    CardAction, CardDoc, FlowGroup, FlowSummary, InputInfo, Manifest, RouteTarget, Warning,
    WarningKind,
};

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub cards_dir: PathBuf,
    pub group_by: Option<GroupBy>,
    pub default_flow: Option<String>,
    pub strict: bool,
}

pub fn scan_cards(config: &ScanConfig) -> Result<Manifest> {
    let mut warnings: Vec<Warning> = Vec::new();
    let mut cards = Vec::new();

    for entry in WalkDir::new(&config.cards_dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let extension = path.extension().and_then(|ext| ext.to_str());
        if extension.is_none_or(|ext| !ext.eq_ignore_ascii_case("json")) {
            continue;
        }

        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(err) => {
                warnings.push(warning(
                    WarningKind::InvalidJson,
                    format!("failed to read {}: {err}", path.display()),
                ));
                continue;
            }
        };

        let value: Value = match serde_json::from_str(&contents) {
            Ok(value) => value,
            Err(err) => {
                if config.strict {
                    bail!("invalid JSON in {}: {err}", path.display());
                }
                warnings.push(warning(
                    WarningKind::InvalidJson,
                    format!("invalid JSON in {}: {err}", path.display()),
                ));
                continue;
            }
        };

        let object = match value.as_object() {
            Some(object) => object,
            None => {
                warnings.push(warning(
                    WarningKind::IgnoredFile,
                    format!("non-object JSON ignored: {}", path.display()),
                ));
                continue;
            }
        };

        if let Some(card_type) = object.get("type").and_then(|value| value.as_str()) {
            if card_type != "AdaptiveCard" {
                warnings.push(warning(
                    WarningKind::IgnoredFile,
                    format!(
                        "non-AdaptiveCard JSON ignored: {} (type={})",
                        path.display(),
                        card_type
                    ),
                ));
                continue;
            }
        } else if !object.contains_key("actions") && !object.contains_key("body") {
            warnings.push(warning(
                WarningKind::IgnoredFile,
                format!("non-card JSON ignored: {}", path.display()),
            ));
            continue;
        }

        let actions_value = object.get("actions").and_then(|value| value.as_array());
        let actions_value = actions_value.map(|array| array.as_slice()).unwrap_or(&[]);

        let mut action_card_ids = Vec::new();
        let mut action_flow_names = Vec::new();
        let mut actions = Vec::new();

        for action in actions_value {
            let action_obj = match action.as_object() {
                Some(action_obj) => action_obj,
                None => {
                    warnings.push(warning(
                        WarningKind::IgnoredFile,
                        format!("ignored non-object action in {}", path.display()),
                    ));
                    continue;
                }
            };

            let action_type = action_obj
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("Unknown")
                .to_string();
            let title = action_obj
                .get("title")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());

            let data = action_obj.get("data").cloned().unwrap_or(Value::Null);
            let data_obj = action_obj.get("data").and_then(|value| value.as_object());

            if let Some(card_id) = data_obj
                .and_then(|obj| obj.get("cardId"))
                .and_then(|value| value.as_str())
            {
                action_card_ids.push(card_id.to_string());
            }

            if let Some(flow) = data_obj
                .and_then(|obj| obj.get("flow"))
                .and_then(|value| value.as_str())
            {
                action_flow_names.push(flow.to_string());
            }

            let action_id = data_obj
                .and_then(|obj| obj.get("action_id"))
                .and_then(|value| value.as_str())
                .map(|s| s.to_string());

            let target = if let Some(step) = data_obj
                .and_then(|obj| obj.get("step"))
                .and_then(|value| value.as_str())
            {
                Some(RouteTarget::Step(step.to_string()))
            } else if let Some(card_id) = data_obj
                .and_then(|obj| obj.get("cardId"))
                .and_then(|value| value.as_str())
            {
                Some(RouteTarget::CardId(card_id.to_string()))
            } else {
                data_obj
                    .and_then(|obj| obj.get("routeToCardId"))
                    .and_then(|value| value.as_str())
                    .map(|card_id| RouteTarget::CardId(card_id.to_string()))
            };

            actions.push(CardAction {
                action_type,
                title,
                action_id,
                target,
                data,
            });
        }

        let rel_path = path
            .strip_prefix(&config.cards_dir)
            .with_context(|| format!("failed to strip prefix for {}", path.display()))?;

        let rel_path_string = rel_path.to_string_lossy().replace('\\', "/").to_string();

        let card_id = resolve_card_id(
            &action_card_ids,
            object,
            &rel_path_string,
            config,
            &mut warnings,
        )?;

        let flow_name =
            resolve_flow_name(&action_flow_names, object, rel_path, config, &mut warnings)?;

        cards.push(CardDoc {
            rel_path: rel_path_string,
            abs_path: path.to_path_buf(),
            card_id,
            flow_name,
            actions,
        });
    }

    if cards.is_empty() {
        if config.strict {
            bail!(
                "no Adaptive Card JSON files found in {}",
                config.cards_dir.display()
            );
        }
        warnings.push(warning(
            WarningKind::IgnoredFile,
            "no Adaptive Card JSON files found".to_string(),
        ));
    }

    let mut flows: BTreeMap<String, Vec<CardDoc>> = BTreeMap::new();
    let mut seen: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for card in cards {
        let flow_name = card.flow_name.clone();
        let flow_seen = seen.entry(flow_name.clone()).or_default();
        if let Some(existing) = flow_seen.get(&card.card_id) {
            let message = format!(
                "duplicate card_id {} in flow {}: {} and {}",
                card.card_id, flow_name, existing, card.rel_path
            );
            if config.strict {
                bail!(message);
            }
            warnings.push(warning(WarningKind::DuplicateCardId, message));
            continue;
        }
        flow_seen.insert(card.card_id.clone(), card.rel_path.clone());
        flows.entry(flow_name).or_default().push(card);
    }

    let mut summaries = Vec::new();
    let mut flow_groups = Vec::new();
    for (flow_name, mut cards) in flows {
        cards.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
        summaries.push(FlowSummary {
            flow_name: flow_name.clone(),
            card_count: cards.len(),
        });
        flow_groups.push(FlowGroup { flow_name, cards });
    }

    let cards_total = cards_count(&flow_groups);

    Ok(Manifest {
        version: 1,
        generated_at: now_rfc3339(),
        input: InputInfo {
            cards_dir: config.cards_dir.clone(),
            group_by: config.group_by,
            default_flow: config.default_flow.clone(),
        },
        flows: flow_groups,
        warnings: warnings.clone(),
        diagnostics: crate::diagnostics::build_diagnostics(
            config.cards_dir.clone(),
            None,
            Vec::new(),
            summaries,
            cards_total,
            warnings.len(),
        ),
    })
}

fn resolve_card_id(
    action_card_ids: &[String],
    object: &serde_json::Map<String, Value>,
    rel_path: &str,
    config: &ScanConfig,
    warnings: &mut Vec<Warning>,
) -> Result<String> {
    if let Some(value) =
        resolve_consistent_value(action_card_ids, "cardId", rel_path, config.strict, warnings)?
    {
        return Ok(value);
    }

    if let Some(value) = object
        .get("greentic")
        .and_then(|value| value.as_object())
        .and_then(|obj| obj.get("cardId"))
        .and_then(|value| value.as_str())
    {
        return Ok(value.to_string());
    }

    let stem = Path::new(rel_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("unable to determine card id for {rel_path}"))?;

    Ok(stem.to_string())
}

fn resolve_flow_name(
    action_flow_names: &[String],
    object: &serde_json::Map<String, Value>,
    rel_path: &Path,
    config: &ScanConfig,
    warnings: &mut Vec<Warning>,
) -> Result<String> {
    if let Some(value) = resolve_consistent_value(
        action_flow_names,
        "flow",
        &rel_path.display().to_string(),
        config.strict,
        warnings,
    )? {
        return Ok(value);
    }

    if let Some(value) = object
        .get("greentic")
        .and_then(|value| value.as_object())
        .and_then(|obj| obj.get("flow"))
        .and_then(|value| value.as_str())
    {
        return Ok(value.to_string());
    }

    if config.group_by == Some(GroupBy::Folder)
        && let Some(folder) = first_folder_component(rel_path)
    {
        return Ok(folder);
    }

    if let Some(default_flow) = config.default_flow.as_ref() {
        return Ok(default_flow.clone());
    }

    if config.strict {
        bail!("unable to resolve flow name for {}", rel_path.display());
    }

    warnings.push(warning(
        WarningKind::MissingFlow,
        format!("flow name missing for {}; using misc", rel_path.display()),
    ));
    Ok("misc".to_string())
}

fn resolve_consistent_value(
    values: &[String],
    label: &str,
    rel_path: &str,
    strict: bool,
    warnings: &mut Vec<Warning>,
) -> Result<Option<String>> {
    if values.is_empty() {
        return Ok(None);
    }

    let mut unique = values.to_vec();
    unique.sort();
    unique.dedup();

    if unique.len() > 1 {
        let message = format!(
            "inconsistent {label} values in {}: {}",
            rel_path,
            unique.join(", ")
        );
        if strict {
            bail!(message);
        }
        warnings.push(warning(WarningKind::Inconsistent, message));
        return Ok(Some(values[0].clone()));
    }

    Ok(Some(unique[0].clone()))
}

fn first_folder_component(rel_path: &Path) -> Option<String> {
    let mut components = rel_path.components();
    let first = components.next()?;
    components.next()?;
    match first {
        std::path::Component::Normal(name) => Some(name.to_string_lossy().to_string()),
        _ => None,
    }
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn cards_count(flows: &[FlowGroup]) -> usize {
    flows.iter().map(|flow| flow.cards.len()).sum()
}
