use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use crate::cli::GroupBy;

#[derive(Debug, Serialize)]
pub struct CardDoc {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub card_id: String,
    pub flow_name: String,
    pub actions: Vec<CardAction>,
}

#[derive(Debug, Serialize)]
pub struct CardAction {
    pub action_type: String,
    pub title: Option<String>,
    pub target: Option<RouteTarget>,
    pub data: Value,
}

#[derive(Debug, Serialize)]
pub enum RouteTarget {
    Step(String),
    CardId(String),
}

#[derive(Debug, Serialize)]
pub struct FlowGroup {
    pub flow_name: String,
    pub cards: Vec<CardDoc>,
}

#[derive(Debug, Serialize)]
pub struct InputInfo {
    pub cards_dir: PathBuf,
    pub group_by: Option<GroupBy>,
    pub default_flow: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Manifest {
    pub version: u8,
    pub generated_at: String,
    pub input: InputInfo,
    pub flows: Vec<FlowGroup>,
    pub warnings: Vec<Warning>,
    pub diagnostics: Diagnostics,
}

#[derive(Debug, Serialize, Clone)]
pub struct Warning {
    pub kind: WarningKind,
    pub message: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    Inconsistent,
    MissingTarget,
    MissingFlow,
    MissingCardId,
    DuplicateCardId,
    InvalidJson,
    IgnoredFile,
    PackOutput,
    Validation,
    Translation,
}

#[derive(Debug, Serialize, Clone)]
pub struct Diagnostics {
    pub workspace_root: PathBuf,
    pub dist_artifact: Option<PathBuf>,
    pub flow_paths: Vec<String>,
    pub cards_processed: usize,
    pub flows: Vec<FlowSummary>,
    pub warnings_count: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct FlowSummary {
    pub flow_name: String,
    pub card_count: usize,
}
