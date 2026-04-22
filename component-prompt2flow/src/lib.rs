mod bindings {
    wit_bindgen::generate!({
        path: "wit/world.wit",
        world: "component",
    });
}

mod jaccard;
mod render_options_card;
mod scorer;
mod tokenize;

use crate::render_options_card::build_options_card;
use crate::scorer::{Score, ScorerKind};
use crate::tokenize::{tokenize, unique_tokens};
use greentic_types::ChannelMessageEnvelope;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub fn describe_payload() -> String {
    include_str!("../component.manifest.json").to_string()
}

pub fn handle_message(input: &str) -> String {
    match serde_json::from_str::<ComponentInput>(input) {
        Ok(parsed) => match run(parsed) {
            Ok(outcome) => serde_json::to_string(&outcome).unwrap_or_else(|_| fallback_outcome_json()),
            Err(err) => error_outcome_json(&err),
        },
        Err(err) => error_outcome_json(&ComponentError::InvalidInput(err.to_string())),
    }
}

#[derive(Deserialize)]
struct ComponentInput {
    config_path: String,
    message: ChannelMessageEnvelope,
    #[serde(default)]
    action: Option<ActionPayload>,
    #[serde(default)]
    session: Value,
    #[serde(default)]
    state: Value,
}

#[derive(Debug)]
enum ComponentError {
    Io(String),
    Parse(String),
    InvalidInput(String),
}

impl ComponentError {
    fn as_str(&self) -> &str {
        match self {
            ComponentError::Io(msg) => msg,
            ComponentError::Parse(msg) => msg,
            ComponentError::InvalidInput(msg) => msg,
        }
    }
}

fn run(input: ComponentInput) -> Result<Outcome, ComponentError> {
    let config = Prompt2FlowConfig::load(Path::new(&input.config_path))?;
    if let Some(action) = input.action {
        if let Some(override_route) = action.route {
            if let Some(flow) = override_route.flow {
                let target = RouteTarget { flow, node: override_route.node };
                return Ok(Outcome::with_route(target));
            }
        }
    }

    let text = input.message.text.as_deref().unwrap_or("").trim();
    if text.is_empty() {
        return Ok(Outcome::empty());
    }

    if config.mode.require_prefix && !matches_prefix(text, &config.mode.prefixes) {
        return Ok(Outcome::empty());
    }

    let query_tokens: Vec<String> = tokenize(text);
    let query_set: HashSet<_> = query_tokens.iter().cloned().collect();
    if query_set.is_empty() {
        return Ok(Outcome::empty());
    }

    let matches = evaluate_intents(&config, &query_set);
    if let Some(route) = pick_route(&matches, &config.mode) {
        return Ok(Outcome::with_route(route));
    }

    let limit = std::cmp::min(matches.len(), config.mode.top_k);
    let card = build_options_card(text, &matches[..limit]);
    Ok(Outcome::with_messages(vec![card]))
}

fn matches_prefix(text: &str, prefixes: &[String]) -> bool {
    let lower = text.to_ascii_lowercase();
    prefixes.iter().any(|prefix| lower.starts_with(&prefix.to_ascii_lowercase()))
}

fn evaluate_intents(config: &Prompt2FlowConfig, query_set: &HashSet<String>) -> Vec<IntentMatch> {
    let scorer = config.scorer.instantiate();
    let mut matches = Vec::new();
    for entry in config.entries() {
        if !entry.anchor_tokens.is_empty() && entry.anchor_tokens.is_disjoint(query_set) {
            continue;
        }
        let Score { value, matched_tokens } = scorer.score(query_set, &entry.tokens);
        matches.push(IntentMatch {
            intent_id: entry.id.clone(),
            title: entry.title.clone(),
            examples: entry.examples.clone(),
            score: value,
            matched_tokens,
            route: entry.route.clone(),
        });
    }
    matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    matches
}

fn pick_route(matches: &[IntentMatch], mode: &ModeConfig) -> Option<RouteTarget> {
    let best = matches.first()?;
    if best.score < mode.min_score {
        return None;
    }
    let second_score = matches.get(1).map_or(0.0, |intent| intent.score);
    if best.score - second_score < mode.min_gap {
        return None;
    }
    Some(best.route.clone())
}

#[derive(Deserialize)]
struct Prompt2FlowConfig {
    #[serde(default)]
    mode: ModeConfig,
    #[serde(default)]
    intents: Vec<IntentConfig>,
    #[serde(default)]
    scorer: ScorerKind,
}

impl Prompt2FlowConfig {
    fn load(path: &Path) -> Result<Self, ComponentError> {
        let data = fs::read_to_string(path).map_err(|err| ComponentError::Io(err.to_string()))?;
        serde_json::from_str(&data).map_err(|err| ComponentError::Parse(err.to_string()))
    }

    fn entries(&self) -> Vec<IntentEntry> {
        self.intents.iter().filter_map(IntentEntry::try_from).collect()
    }
}

#[derive(Deserialize, Default)]
struct ModeConfig {
    #[serde(default = "default_require_prefix")]
    require_prefix: bool,
    #[serde(default = "default_prefixes")]
    prefixes: Vec<String>,
    #[serde(default = "default_min_score")]
    min_score: f64,
    #[serde(default = "default_min_gap")]
    min_gap: f64,
    #[serde(default = "default_top_k")]
    top_k: usize,
}

fn default_require_prefix() -> bool {
    true
}

fn default_prefixes() -> Vec<String> {
    vec!["go:".to_string(), "/".to_string()]
}

fn default_min_score() -> f64 {
    0.35
}

fn default_min_gap() -> f64 {
    0.1
}

fn default_top_k() -> usize {
    3
}

#[derive(Deserialize)]
struct IntentConfig {
    id: String,
    title: String,
    route: IntentRouteConfig,
    #[serde(default)]
    examples: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    anchors: Vec<String>,
}

#[derive(Deserialize)]
struct IntentRouteConfig {
    #[serde(default)]
    flow: Option<String>,
    #[serde(default)]
    node: Option<String>,
}

#[derive(Clone)]
struct IntentEntry {
    id: String,
    title: String,
    route: RouteTarget,
    tokens: HashSet<String>,
    anchor_tokens: HashSet<String>,
    examples: Vec<String>,
}

impl IntentEntry {
    fn try_from(cfg: &IntentConfig) -> Option<Self> {
        let flow = cfg.route.flow.clone()?;
        if flow.trim().is_empty() || cfg.id.trim().is_empty() {
            return None;
        }
        let mut tokens: HashSet<String> = HashSet::new();
        tokens.extend(tokenize(&cfg.title));
        for example in &cfg.examples {
            tokens.extend(tokenize(example));
        }
        for keyword in &cfg.keywords {
            tokens.extend(tokenize(keyword));
        }
        let anchor_tokens = cfg
            .anchors
            .iter()
            .flat_map(|anchor| tokenize(anchor))
            .collect();
        Some(Self {
            id: cfg.id.clone(),
            title: cfg.title.clone(),
            route: RouteTarget { flow, node: cfg.route.node.clone() },
            tokens,
            anchor_tokens,
            examples: cfg.examples.clone(),
        })
    }
}

#[derive(Clone)]
pub struct IntentMatch {
    pub intent_id: String,
    pub title: String,
    pub examples: Vec<String>,
    pub score: f64,
    pub matched_tokens: Vec<String>,
    pub route: RouteTarget,
}

impl IntentMatch {
    fn route_payload(&self) -> Value {
        json!({
            "flow": self.route.flow,
            "node": self.route.node,
        })
    }
}

#[derive(Clone)]
struct RouteTarget {
    flow: String,
    node: Option<String>,
}

impl RouteTarget {
    fn as_route_string(&self) -> String {
        if let Some(node) = &self.node {
            format!("{}/{}", self.flow, node)
        } else {
            self.flow.clone()
        }
    }
}

#[derive(Deserialize)]
struct ActionPayload {
    #[serde(default)]
    route: Option<RouteOverride>,
}

#[derive(Deserialize)]
struct RouteOverride {
    #[serde(default)]
    flow: Option<String>,
    #[serde(default)]
    node: Option<String>,
}

#[derive(Serialize)]
struct Outcome {
    messages: Vec<Value>,
    #[serde(rename = "state_patch")]
    state_patch: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    route: Option<String>,
}

impl Outcome {
    fn empty() -> Self {
        Self {
            messages: Vec::new(),
            state_patch: empty_object(),
            route: None,
        }
    }

    fn with_messages(messages: Vec<Value>) -> Self {
        Self { messages, ..Self::empty() }
    }

    fn with_route(target: RouteTarget) -> Self {
        Self {
            route: Some(target.as_route_string()),
            ..Self::empty()
        }
    }
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

fn fallback_outcome_json() -> String {
    json!({
        "messages": [],
        "state_patch": {},
        "route": null
    })
    .to_string()
}

fn error_outcome_json(err: &ComponentError) -> String {
    let message = format!("prompt2flow error: {}", err.as_str());
    let card = json!({
        "type": "MessageCard",
        "tier": "advanced",
        "payload": {
            "adaptive_card": {
                "type": "AdaptiveCard",
                "version": "1.4",
                "body": [
                    {"type": "TextBlock", "text": message, "color": "attention", "wrap": true}
                ]
            }
        }
    });
    serde_json::to_string(&Outcome::with_messages(vec![card])).unwrap_or_else(|_| fallback_outcome_json())
}

struct Component;

impl bindings::Guest for Component {
    fn describe() -> String {
        describe_payload()
    }

    fn handle(input: String) -> String {
        handle_message(&input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_types::{ChannelMessageEnvelope, EnvId, TenantCtx, TenantId};
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn intent_config(
        id: &str,
        title: &str,
        flow: &str,
        node: Option<&str>,
        examples: &[&str],
        keywords: &[&str],
        anchors: &[&str],
    ) -> Value {
        json!({
            "id": id,
            "title": title,
            "route": {
                "flow": flow,
                "node": node,
            },
            "examples": examples,
            "keywords": keywords,
            "anchors": anchors,
        })
    }

    fn write_config(config: &Value) -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().expect("create temp dir for config");
        let config_path = temp_dir.path().join("prompt2flow.json");
        fs::write(&config_path, serde_json::to_string_pretty(config).unwrap())
            .expect("write config");
        (temp_dir, config_path)
    }

    fn sample_envelope(text: &str) -> ChannelMessageEnvelope {
        ChannelMessageEnvelope {
            id: "msg-1".to_string(),
            tenant: TenantCtx::new(
                EnvId::try_from("dev").unwrap(),
                TenantId::try_from("tenant-1").unwrap(),
            ),
            channel: "test".to_string(),
            session_id: "session-1".to_string(),
            reply_scope: None,
            from: None,
            to: Vec::new(),
            correlation_id: None,
            text: Some(text.to_string()),
            attachments: Vec::new(),
            metadata: BTreeMap::new(),
            extensions: BTreeMap::new(),
        }
    }

    fn run_with_config(config: &Value, text: &str, action: Option<ActionPayload>) -> Outcome {
        let (_temp_dir, config_path) = write_config(config);
        let input = ComponentInput {
            config_path: config_path.to_string_lossy().into_owned(),
            message: sample_envelope(text),
            action,
            session: json!({}),
            state: json!({}),
        };
        run(input).expect("run prompt2flow")
    }

    #[test]
    fn prefix_gating_blocks_unprefixed_text() {
        let config = json!({
            "mode": {
                "require_prefix": true,
                "prefixes": ["go:", "/"]
            },
            "intents": [
                intent_config("setup", "Setup Flow", "setup", Some("start"), &["setup"], &[], &[])
            ]
        });

        let outcome = run_with_config(&config, "configure webex", None);
        assert!(outcome.route.is_none());
        assert!(outcome.messages.is_empty());
    }

    #[test]
    fn anchors_must_match_before_routing() {
        let config = json!({
            "intents": [
                intent_config("webex", "Webex Setup", "setup", None, &["webex"], &[], &["webex"]),
            ]
        });

        let without_anchor = run_with_config(&config, "go: configure zoom", None);
        assert!(without_anchor.route.is_none());

        let with_anchor = run_with_config(&config, "go: webex configuration", None);
        assert_eq!(with_anchor.route, Some("setup".to_string()));
    }

    #[test]
    fn requires_score_gap_between_top_two() {
        let config = json!({
            "mode": { "min_gap": 0.25 },
            "intents": [
                intent_config("alpha", "Common Flow", "alpha", None, &["common"], &["common"], &[]),
                intent_config("beta", "Common Flow B", "beta", None, &["common"], &["common"], &[]),
            ]
        });

        let outcome = run_with_config(&config, "go: common", None);
        assert!(outcome.route.is_none());
        assert!(!outcome.messages.is_empty());
    }

    #[test]
    fn action_route_override_short_circuits_scoring() {
        let config = json!({
            "intents": [
                intent_config("default", "Default Flow", "default", Some("start"), &["default"], &[], &[])
            ]
        });

        let action = ActionPayload {
            route: Some(RouteOverride { flow: Some("override".to_string()), node: Some("start".to_string()) }),
        };

        let outcome = run_with_config(&config, "go: default", Some(action));
        assert_eq!(outcome.route, Some("override/start".to_string()));
        assert!(outcome.messages.is_empty());
    }
}
