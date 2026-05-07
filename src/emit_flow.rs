use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::diagnostics::warning;
use crate::graph::{FlowGraph, FlowNode};
use crate::ir::{Warning, WarningKind};

const BEGIN_MARKER: &str = "# BEGIN GENERATED (cards2pack)";
const END_MARKER: &str = "# END GENERATED (cards2pack)";
const COMPONENT_REF: &str = "oci://ghcr.io/greenticai/components/component-adaptive-card:stable";

pub fn emit_flow(
    graph: &FlowGraph,
    workspace_root: &Path,
    strict: bool,
    custom_langs: Option<&[String]>,
) -> Result<(PathBuf, Vec<Warning>)> {
    let flows_dir = workspace_root.join("flows");
    fs::create_dir_all(&flows_dir)
        .with_context(|| format!("failed to create {}", flows_dir.display()))?;

    let path = flows_dir.join("main.ygtc");
    let (generated, warnings) =
        generate_flow_with_cli(graph, workspace_root, strict, custom_langs)?;
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
    custom_langs: Option<&[String]>,
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
    let (order, start_node) = resolve_node_order(graph);
    let total_nodes = order.len();
    let mut created: BTreeSet<String> = BTreeSet::new();
    // Collect conditional routing for post-processing: node_id → full routes with action_ids.
    let mut conditional_routing: BTreeMap<String, Vec<ResolvedRoute>> = BTreeMap::new();

    // Load i18n en.json for inlining into card_spec (flow engine doesn't resolve assets).
    let i18n_inline = load_i18n_inline(workspace_root);

    eprintln!(
        "[flow] Adding {total_nodes} nodes to flow '{}'...",
        graph.flow_name
    );

    for (idx, node_id) in order.iter().enumerate() {
        let node = graph
            .nodes
            .get(node_id)
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
        let payload = build_card_payload(node_id, &card_path_value, custom_langs, &i18n_inline);

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

        // Track nodes that need conditional routing for post-processing.
        if routes.iter().any(|r| r.action_id.is_some()) {
            // Collect ALL routes for this node (including skipped forward refs).
            let all_routes: Vec<ResolvedRoute> = node
                .routes
                .iter()
                .map(|r| ResolvedRoute {
                    target: r.target.clone(),
                    action_id: r.action_id.clone(),
                })
                .collect();
            conditional_routing.insert(node_id.clone(), all_routes);
        }

        push_routing_flags(&mut args, node_id, &routes, workspace_root, &mut warnings);

        // Auto-answer the component wizard questions:
        // 1. default_source → "2" (asset)
        // 2. default_card_asset → card asset path
        // 3. multilingual → "" (accept default: true)
        // 4. language_mode → "1" (all) or "2" (custom) + langs
        let wizard_answers = match custom_langs {
            Some(langs) => format!("2\n{card_path_value}\n\n2\n{}\n", langs.join(",")),
            None => format!("2\n{card_path_value}\n\n1\n"),
        };
        run_greentic_flow_with_stdin(&args, &wizard_answers)?;
        eprint!("\r[flow] Progress: {}/{total_nodes}", idx + 1);
        let _ = std::io::stderr().flush();
        created.insert(node_id.clone());
    }
    eprintln!();

    let mut contents = fs::read_to_string(&tmp_flow)
        .with_context(|| format!("failed to read {}", tmp_flow.display()))?;

    // Fix the start: field to point to the real root node (not the first-created leaf).
    if let Some(ref root) = start_node {
        contents = fix_start_node(&contents, root);
    }

    // Post-process: inject conditional routing with `condition` expressions.
    if !conditional_routing.is_empty() {
        contents = inject_conditional_routing(&contents, &conditional_routing);
    }

    Ok((contents.trim_end().to_string(), warnings))
}

struct ResolvedRoute {
    target: String,
    action_id: Option<String>,
}

fn resolve_routes(
    node: &FlowNode,
    created: &BTreeSet<String>,
) -> (Vec<ResolvedRoute>, Vec<String>) {
    let mut routes = Vec::new();
    let mut skipped = Vec::new();

    for route in &node.routes {
        if created.contains(&route.target) {
            routes.push(ResolvedRoute {
                target: route.target.clone(),
                action_id: route.action_id.clone(),
            });
        } else {
            skipped.push(route.target.clone());
        }
    }

    (routes, skipped)
}

/// Returns (creation_order, start_node).
/// Creation order is leaf-first so that `greentic-flow add-step` targets exist.
/// Start node is the real root (0 incoming edges, prefer "welcome").
fn resolve_node_order(graph: &FlowGraph) -> (Vec<String>, Option<String>) {
    // outgoing_count: used for leaf-first creation order.
    let mut outgoing_count: BTreeMap<String, usize> =
        graph.nodes.keys().map(|key| (key.clone(), 0)).collect();
    // incoming_count: used for detecting the real root/start node.
    let mut incoming_count: BTreeMap<String, usize> =
        graph.nodes.keys().map(|key| (key.clone(), 0)).collect();
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for node in graph.nodes.values() {
        for route in &node.routes {
            if !graph.nodes.contains_key(&route.target) {
                continue;
            }
            *outgoing_count.entry(node.name.clone()).or_insert(0) += 1;
            *incoming_count.entry(route.target.clone()).or_insert(0) += 1;
            dependents
                .entry(route.target.clone())
                .or_default()
                .push(node.name.clone());
        }
    }

    // Leaf-first creation order (nodes with 0 outgoing edges first).
    let mut queue: Vec<String> = outgoing_count
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
                if let Some(entry) = outgoing_count.get_mut(child) {
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

    // Detect the real start node: 0 incoming edges, prefer "welcome".
    let roots: Vec<String> = incoming_count
        .iter()
        .filter_map(|(node, count)| {
            if *count == 0 {
                Some(node.clone())
            } else {
                None
            }
        })
        .collect();

    // Determine the start node. Cards may have nested ActionSets whose routes aren't
    // tracked in incoming edges, so always prefer a "welcome" node as the start.
    let start_node = graph
        .nodes
        .keys()
        .find(|n| n.contains("welcome"))
        .cloned()
        .or_else(|| roots.first().cloned())
        .or_else(|| graph.nodes.keys().next().cloned());
    (ordered, start_node)
}

fn build_card_payload(
    node_id: &str,
    card_path: &str,
    custom_langs: Option<&[String]>,
    i18n_inline: &Option<serde_json::Value>,
) -> String {
    let mut input = serde_json::Map::new();
    input.insert("card_source".to_string(), json!("asset"));
    let mut card_spec = json!({
        "asset_path": card_path,
        "i18n_bundle_path": "assets/i18n"
    });
    // Inline i18n translations so the component doesn't depend on host asset resolution.
    if let Some(inline) = i18n_inline {
        card_spec["i18n_inline"] = inline.clone();
    }
    input.insert("card_spec".to_string(), card_spec);
    input.insert("mode".to_string(), json!("renderAndValidate"));
    input.insert("node_id".to_string(), json!(node_id));
    input.insert("payload".to_string(), json!({}));
    input.insert("session".to_string(), json!({}));
    input.insert("state".to_string(), json!({}));
    input.insert("validation_mode".to_string(), json!("warn"));

    // i18n config for the component runtime.
    input.insert("multilingual".to_string(), json!(true));
    if let Some(langs) = custom_langs {
        input.insert("language_mode".to_string(), json!("custom"));
        // Always include "en" as source locale.
        let mut all_locales: Vec<&str> = vec!["en"];
        for lang in langs {
            if lang != "en" {
                all_locales.push(lang);
            }
        }
        input.insert("supported_locales".to_string(), json!(all_locales));
    } else {
        input.insert("language_mode".to_string(), json!("all"));
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
    routes: &[ResolvedRoute],
    _workspace_root: &Path,
    warnings: &mut Vec<Warning>,
) {
    if routes.is_empty() {
        warnings.push(warning(
            WarningKind::MissingTarget,
            format!("no routes for {}; using routing-out", node_id),
        ));
        args.push("--routing-out".to_string());
        return;
    }

    // Use simple routing for greentic-flow add-step (it doesn't support `condition`).
    // Conditional routing with action_ids is applied in post-processing.
    let targets: Vec<String> = routes.iter().map(|r| r.target.clone()).collect();
    if targets.len() == 1 {
        args.push("--routing-next".to_string());
        args.push(targets[0].clone());
    } else {
        args.push("--routing-multi-to".to_string());
        args.push(targets.join(","));
    }
}

fn run_greentic_flow(args: &[&str]) -> Result<()> {
    let output = Command::new("greentic-flow")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .with_context(|| format!("failed to run greentic-flow {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "greentic-flow command failed: greentic-flow {}\n{}",
            args.join(" "),
            stderr.trim_end()
        );
    }
    Ok(())
}

fn run_greentic_flow_with_stdin(args: &[String], stdin_data: &str) -> Result<()> {
    let mut child = Command::new("greentic-flow")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn greentic-flow {}", args.join(" ")))?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_data.as_bytes());
    }

    let output = child
        .wait_with_output()
        .with_context(|| format!("failed to wait for greentic-flow {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "greentic-flow command failed: greentic-flow {}\n{}",
            args.join(" "),
            stderr.trim_end()
        );
    }
    Ok(())
}

/// Replaces simple multi-target routing with conditional routing based on action_ids.
/// Rewrites the YAML `routing:` section for nodes that have conditional routes.
fn inject_conditional_routing(
    contents: &str,
    conditional_routing: &BTreeMap<String, Vec<ResolvedRoute>>,
) -> String {
    let lines: Vec<&str> = contents.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Detect a node header like "  welcome_card:" at indent level 2.
        let trimmed = line.trim_start();
        if let Some(node_id) = trimmed.strip_suffix(':')
            && let Some(routes) = conditional_routing.get(node_id)
        {
            // Found a node with conditional routing. Copy the node header.
            result.push(line.to_string());
            i += 1;

            // Find and replace the `routing:` block for this node.
            let node_indent = line.len() - trimmed.len();
            while i < lines.len() {
                let inner = lines[i];
                let inner_trimmed = inner.trim_start();
                let inner_indent = inner.len() - inner_trimmed.len();

                // If we've exited the node's scope, stop.
                if !inner_trimmed.is_empty() && inner_indent <= node_indent {
                    break;
                }

                if inner_trimmed.starts_with("routing:") {
                    // Replace the routing block.
                    let routing_indent = inner_indent;
                    let entry_indent = " ".repeat(routing_indent);
                    let field_indent = " ".repeat(routing_indent + 2);

                    result.push(format!("{}routing:", " ".repeat(routing_indent)));
                    for route in routes {
                        if let Some(action_id) = &route.action_id {
                            result.push(format!(
                                    "{entry_indent}- condition: \"response.action == \\\"{action_id}\\\"\""
                                ));
                            result.push(format!("{field_indent}to: {}", route.target));
                        } else {
                            result.push(format!("{entry_indent}- to: {}", route.target));
                        }
                    }
                    i += 1;

                    // Skip old routing entries: list items at routing_indent (`- ...`)
                    // and any deeper continuation lines.
                    while i < lines.len() {
                        let next = lines[i];
                        let next_trimmed = next.trim_start();
                        let next_indent = next.len() - next_trimmed.len();
                        if next_trimmed.is_empty() {
                            i += 1;
                        } else if next_indent > routing_indent {
                            // Deeper content (part of old routing)
                            i += 1;
                        } else if next_indent == routing_indent && next_trimmed.starts_with("- ") {
                            // Same-level list entry (old routing item)
                            i += 1;
                        } else {
                            break;
                        }
                    }
                } else {
                    result.push(inner.to_string());
                    i += 1;
                }
            }
            continue;
        }

        result.push(line.to_string());
        i += 1;
    }

    result.join("\n") + "\n"
}

/// Load i18n en.json from the workspace and wrap it as `{"en": {...}}` for inline embedding.
fn load_i18n_inline(workspace_root: &Path) -> Option<serde_json::Value> {
    let en_path = workspace_root.join("assets").join("i18n").join("en.json");
    let raw = fs::read_to_string(&en_path).ok()?;
    let bundle: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(json!({ "en": bundle }))
}

fn fix_start_node(contents: &str, root: &str) -> String {
    // Replace `start: <whatever>` with `start: <root>` in the YAML.
    let mut result = String::with_capacity(contents.len());
    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("start:") {
            let indent = &line[..line.len() - trimmed.len()];
            result.push_str(&format!("{indent}start: {root}"));
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    result
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
