use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::diagnostics::warning;
use crate::graph::{FlowGraph, FlowNode};
use crate::ir::{Warning, WarningKind};

const BEGIN_MARKER: &str = "# BEGIN GENERATED (cards2pack)";
const END_MARKER: &str = "# END GENERATED (cards2pack)";
const COMPONENT_REF: &str = "oci://ghcr.io/greenticai/components/component-adaptive-card:latest";

pub fn emit_flow(
    graph: &FlowGraph,
    workspace_root: &Path,
    strict: bool,
) -> Result<(PathBuf, Vec<Warning>)> {
    let flows_dir = workspace_root.join("flows");
    fs::create_dir_all(&flows_dir)
        .with_context(|| format!("failed to create {}", flows_dir.display()))?;

    let path = flows_dir.join("main.ygtc");
    let (generated, warnings) = generate_flow_with_cli(graph, workspace_root, strict)?;
    let block = format!("{BEGIN_MARKER}\n{generated}\n{END_MARKER}\n");

    let next_contents = if path.exists() {
        let existing = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if path.ends_with("main.ygtc") {
            merge_main_flow(&existing, &block)
        } else {
            replace_generated_block(&existing, &block)
        }
    } else if path.ends_with("main.ygtc") {
        format!(
            "{block}\n# Developer space below (preserved on regen; keep it commented)\n",
            block = block
        )
    } else {
        format!(
            "{block}\n# Developer space below (preserved on regen)\n",
            block = block
        )
    };

    fs::write(&path, next_contents)
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok((path, warnings))
}

fn generate_flow_with_cli(
    graph: &FlowGraph,
    workspace_root: &Path,
    strict: bool,
) -> Result<(String, Vec<Warning>)> {
    let tmp_dir = workspace_root.join(".cards2pack").join("tmp");
    fs::create_dir_all(&tmp_dir)
        .with_context(|| format!("failed to create {}", tmp_dir.display()))?;
    let tmp_flow = tmp_dir.join(format!("{}.flow.yaml", graph.flow_name));

    run_greentic_flow(&[
        "new",
        "--flow",
        tmp_flow.to_string_lossy().as_ref(),
        "--id",
        graph.flow_name.as_str(),
        "--type",
        "messaging",
        "--force",
    ])?;

    let mut warnings = Vec::new();
    let order = resolve_node_order(graph);
    let mut created: BTreeSet<String> = BTreeSet::new();

    for node_id in order {
        let node = graph
            .nodes
            .get(&node_id)
            .ok_or_else(|| anyhow::anyhow!("missing node {node_id}"))?;

        let (routes, skipped) = resolve_routes(node, &created);
        if !skipped.is_empty() {
            if strict {
                bail!(
                    "unable to emit routing for {} due to cycle/ordering: {}",
                    node_id,
                    skipped.join(", ")
                );
            }
            for target in skipped {
                warnings.push(warning(
                    WarningKind::Inconsistent,
                    format!(
                        "routing from {} to {} omitted due to ordering; check for cycles",
                        node_id, target
                    ),
                ));
            }
        }

        let card_path_value = if let Some(card_path) = &node.card_path {
            card_path.clone()
        } else {
            warnings.push(warning(
                WarningKind::MissingTarget,
                format!("stub node {} emitted without card_path", node_id),
            ));
            "TODO".to_string()
        };
        let needs_interaction = !node.routes.is_empty();
        let payload = build_card_payload(&node_id, &card_path_value, needs_interaction);

        let mut args = vec![
            "add-step".to_string(),
            "--flow".to_string(),
            tmp_flow.to_string_lossy().to_string(),
            "--node-id".to_string(),
            node_id.clone(),
            "--component".to_string(),
            COMPONENT_REF.to_string(),
            "--operation".to_string(),
            "card".to_string(),
            "--payload".to_string(),
            payload,
            "--allow-cycles".to_string(),
        ];

        push_routing_flags(&mut args, &node_id, &routes, &mut warnings);

        run_greentic_flow_strings(&args)?;
        created.insert(node_id);
    }

    let contents = fs::read_to_string(&tmp_flow)
        .with_context(|| format!("failed to read {}", tmp_flow.display()))?;

    Ok((contents.trim_end().to_string(), warnings))
}

fn resolve_routes(node: &FlowNode, created: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
    let mut routes = Vec::new();
    let mut skipped = Vec::new();

    for route in &node.routes {
        if created.contains(&route.target) {
            routes.push(route.target.clone());
        } else {
            skipped.push(route.target.clone());
        }
    }

    (routes, skipped)
}

fn resolve_node_order(graph: &FlowGraph) -> Vec<String> {
    let mut indegree: BTreeMap<String, usize> =
        graph.nodes.keys().map(|key| (key.clone(), 0)).collect();
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for node in graph.nodes.values() {
        for route in &node.routes {
            if !graph.nodes.contains_key(&route.target) {
                continue;
            }
            *indegree.entry(node.name.clone()).or_insert(0) += 1;
            dependents
                .entry(route.target.clone())
                .or_default()
                .push(node.name.clone());
        }
    }

    let mut queue: Vec<String> = indegree
        .iter()
        .filter_map(|(node, count)| {
            if *count == 0 {
                Some(node.clone())
            } else {
                None
            }
        })
        .collect();
    queue.sort();

    let mut ordered = Vec::new();
    while let Some(node) = queue.pop() {
        ordered.push(node.clone());
        if let Some(children) = dependents.get(&node) {
            for child in children {
                if let Some(entry) = indegree.get_mut(child) {
                    *entry = entry.saturating_sub(1);
                    if *entry == 0 {
                        queue.push(child.clone());
                        queue.sort();
                    }
                }
            }
        }
    }

    if ordered.len() != graph.nodes.len() {
        let mut remaining: Vec<String> = graph
            .nodes
            .keys()
            .filter(|key| !ordered.contains(key))
            .cloned()
            .collect();
        remaining.sort();
        ordered.extend(remaining);
    }

    ordered
}

fn build_card_payload(node_id: &str, card_path: &str, needs_interaction: bool) -> String {
    let mut input = serde_json::Map::new();
    input.insert("card_source".to_string(), json!("asset"));
    input.insert("card_spec".to_string(), json!({ "asset_path": card_path }));
    input.insert("mode".to_string(), json!("renderAndValidate"));
    input.insert("node_id".to_string(), json!(node_id));
    input.insert("payload".to_string(), json!({}));
    input.insert("session".to_string(), json!({}));
    input.insert("state".to_string(), json!({}));
    input.insert("validation_mode".to_string(), json!("warn"));
    if needs_interaction {
        input.insert(
            "interaction".to_string(),
            json!({
                "action_id": "action-1",
                "card_instance_id": node_id,
                "interaction_type": "Submit",
                "raw_inputs": {}
            }),
        );
    }
    let call_payload = serde_json::Value::Object(input.clone());
    let mut call = serde_json::Map::new();
    call.insert("op".to_string(), json!("render"));
    call.insert("payload".to_string(), call_payload);
    call.insert("metadata".to_string(), json!([]));

    input.insert("call".to_string(), serde_json::Value::Object(call));
    serde_json::Value::Object(input).to_string()
}

fn push_routing_flags(
    args: &mut Vec<String>,
    node_id: &str,
    routes: &[String],
    warnings: &mut Vec<Warning>,
) {
    match routes.len() {
        0 => {
            warnings.push(warning(
                WarningKind::MissingTarget,
                format!("no routes for {}; using routing-out", node_id),
            ));
            args.push("--routing-out".to_string());
        }
        1 => {
            args.push("--routing-next".to_string());
            args.push(routes[0].clone());
        }
        _ => {
            args.push("--routing-multi-to".to_string());
            args.push(routes.join(","));
        }
    }
}

fn run_greentic_flow(args: &[&str]) -> Result<()> {
    let status = Command::new("greentic-flow")
        .args(args)
        .status()
        .with_context(|| format!("failed to run greentic-flow {}", args.join(" ")))?;
    if !status.success() {
        bail!(
            "greentic-flow command failed: greentic-flow {}",
            args.join(" ")
        );
    }
    Ok(())
}

fn run_greentic_flow_strings(args: &[String]) -> Result<()> {
    let status = Command::new("greentic-flow")
        .args(args)
        .status()
        .with_context(|| format!("failed to run greentic-flow {}", args.join(" ")))?;
    if !status.success() {
        bail!(
            "greentic-flow command failed: greentic-flow {}",
            args.join(" ")
        );
    }
    Ok(())
}

fn replace_generated_block(existing: &str, block: &str) -> String {
    let start = existing.find(BEGIN_MARKER);
    let end = existing.find(END_MARKER);

    match (start, end) {
        (Some(start), Some(end)) if end > start => {
            let after_end = existing[end..].find('\n').map(|idx| end + idx + 1);
            let before = &existing[..start];
            let after = after_end.map_or("", |idx| &existing[idx..]);
            format!("{before}{block}{after}")
        }
        _ => {
            if existing.trim().is_empty() {
                format!("{block}\n# Developer space below (preserved on regen)\n")
            } else {
                format!("{block}\n{existing}")
            }
        }
    }
}

fn merge_main_flow(existing: &str, block: &str) -> String {
    let dev_section = extract_dev_section(existing);
    let commented = comment_block(dev_section);
    if commented.is_empty() {
        format!("{block}\n# Developer space below (preserved on regen; keep it commented)\n")
    } else {
        format!(
            "{block}\n# Developer space below (preserved on regen; keep it commented)\n{commented}\n"
        )
    }
}

fn extract_dev_section(existing: &str) -> &str {
    let start = existing.find(BEGIN_MARKER);
    let end = existing.find(END_MARKER);
    if let (Some(start), Some(end)) = (start, end)
        && end > start
        && let Some(after_end) = existing[end..].find('\n').map(|idx| end + idx + 1)
    {
        return existing[after_end..].trim();
    }
    existing.trim()
}

fn comment_block(input: &str) -> String {
    if input.trim().is_empty() {
        return String::new();
    }
    input
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                "#".to_string()
            } else {
                format!("# {}", line)
            }
        })
        .collect::<Vec<String>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::replace_generated_block;

    #[test]
    fn replaces_existing_block() {
        let existing = "# BEGIN GENERATED (cards2pack)\nold\n# END GENERATED (cards2pack)\nkeep\n";
        let block = "# BEGIN GENERATED (cards2pack)\nnew\n# END GENERATED (cards2pack)\n";
        let updated = replace_generated_block(existing, block);
        assert!(updated.contains("new"));
        assert!(updated.contains("keep"));
        assert!(!updated.contains("old"));
    }
}

