use crate::ir::{Diagnostics, FlowSummary, Warning, WarningKind};

pub fn warning(kind: WarningKind, message: impl Into<String>) -> Warning {
    Warning {
        kind,
        message: message.into(),
    }
}

pub fn summarize(diagnostics: &Diagnostics, warnings: &[Warning]) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "Workspace: {}\n",
        diagnostics.workspace_root.display()
    ));
    if let Some(dist) = diagnostics.dist_artifact.as_ref() {
        output.push_str(&format!("Pack: {}\n", dist.display()));
    }
    output.push_str(&format!(
        "Cards processed: {}\n",
        diagnostics.cards_processed
    ));

    output.push_str("Flows:\n");
    if diagnostics.flows.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for flow in &diagnostics.flows {
            output.push_str(&format!(
                "  - {} ({} cards)\n",
                flow.flow_name, flow.card_count
            ));
        }
    }

    output.push_str("Generated flow files:\n");
    if diagnostics.flow_paths.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for path in &diagnostics.flow_paths {
            output.push_str(&format!("  - {}\n", path));
        }
    }

    output.push_str(&format!("Warnings: {}\n", diagnostics.warnings_count));

    for warning in warnings.iter().take(5) {
        output.push_str(&format!(
            "  - [{}] {}\n",
            format_kind(&warning.kind),
            warning.message
        ));
    }

    output.trim_end().to_string()
}

fn format_kind(kind: &WarningKind) -> &'static str {
    match kind {
        WarningKind::Inconsistent => "inconsistent",
        WarningKind::MissingTarget => "missing_target",
        WarningKind::MissingFlow => "missing_flow",
        WarningKind::MissingCardId => "missing_card_id",
        WarningKind::DuplicateCardId => "duplicate_card_id",
        WarningKind::InvalidJson => "invalid_json",
        WarningKind::IgnoredFile => "ignored_file",
        WarningKind::PackOutput => "pack_output",
        WarningKind::Validation => "validation",
        WarningKind::Translation => "translation",
    }
}

pub fn build_diagnostics(
    workspace_root: std::path::PathBuf,
    dist_artifact: Option<std::path::PathBuf>,
    flow_paths: Vec<String>,
    flows: Vec<FlowSummary>,
    cards_processed: usize,
    warnings_count: usize,
) -> Diagnostics {
    Diagnostics {
        workspace_root,
        dist_artifact,
        flow_paths,
        cards_processed,
        flows,
        warnings_count,
    }
}
