use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};

use crate::diagnostics::warning;
use crate::ir::{FlowGroup, RouteTarget, Warning, WarningKind};

#[derive(Debug)]
pub struct FlowGraph {
    pub flow_name: String,
    pub nodes: BTreeMap<String, FlowNode>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug)]
pub struct FlowNode {
    pub name: String,
    pub card_path: Option<String>,
    pub routes: Vec<RouteEdge>,
    pub stub: bool,
}

#[derive(Debug)]
pub struct RouteEdge {
    pub key: String,
    pub target: String,
    pub action_id: Option<String>,
}

pub fn build_flow_graph(group: &FlowGroup, strict: bool) -> Result<FlowGraph> {
    let mut nodes: BTreeMap<String, FlowNode> = BTreeMap::new();
    let mut warnings: Vec<Warning> = Vec::new();

    for card in &group.cards {
        nodes
            .entry(card.card_id.clone())
            .or_insert_with(|| FlowNode {
                name: card.card_id.clone(),
                card_path: Some(format!("assets/cards/{}", card.rel_path)),
                routes: Vec::new(),
                stub: false,
            });
    }

    for card in &group.cards {
        let mut used_keys: BTreeSet<String> = BTreeSet::new();
        let mut routes = Vec::new();

        for (index, action) in card.actions.iter().enumerate() {
            let target_name = match &action.target {
                Some(RouteTarget::Step(name)) => name.clone(),
                Some(RouteTarget::CardId(name)) => name.clone(),
                // Actions with action_id but no explicit target route back to self.
                None if action.action_id.is_some() => card.card_id.clone(),
                None => continue,
            };

            if !nodes.contains_key(&target_name) {
                if strict {
                    bail!(
                        "missing target {} referenced from card {} in flow {}",
                        target_name,
                        card.card_id,
                        group.flow_name
                    );
                }
                warnings.push(warning(
                    WarningKind::MissingTarget,
                    format!(
                        "missing target {} referenced from card {} in flow {}; creating stub",
                        target_name, card.card_id, group.flow_name
                    ),
                ));
                nodes.insert(
                    target_name.clone(),
                    FlowNode {
                        name: target_name.clone(),
                        card_path: None,
                        routes: Vec::new(),
                        stub: true,
                    },
                );
            }

            let mut key = if !target_name.is_empty() {
                target_name.clone()
            } else if let Some(title) = action.title.as_ref()
                && !title.is_empty()
            {
                title.clone()
            } else {
                format!("action-{}", index + 1)
            };
            if used_keys.contains(&key) {
                let mut suffix = 2;
                while used_keys.contains(&format!("{}-{}", key, suffix)) {
                    suffix += 1;
                }
                let new_key = format!("{}-{}", key, suffix);
                warnings.push(warning(
                    WarningKind::Inconsistent,
                    format!(
                        "duplicate route key {} in card {}; renamed to {}",
                        key, card.card_id, new_key
                    ),
                ));
                key = new_key;
            }
            used_keys.insert(key.clone());

            routes.push(RouteEdge {
                key,
                target: target_name,
                action_id: action.action_id.clone(),
            });
        }

        if let Some(node) = nodes.get_mut(&card.card_id) {
            node.routes.extend(routes);
        }
    }

    Ok(FlowGraph {
        flow_name: group.flow_name.clone(),
        nodes,
        warnings,
    })
}
