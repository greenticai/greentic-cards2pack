use std::fs;
use std::path::{Path, PathBuf};

use greentic_cards2pack::emit_flow::emit_flow;
use greentic_cards2pack::graph::build_flow_graph;
use greentic_cards2pack::scan::{ScanConfig, scan_cards};
use tempfile::TempDir;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cards/flow_emit")
}

fn copy_fixture(rel: &str, dest_root: &Path) -> PathBuf {
    let source = fixtures_root().join(rel);
    let dest = dest_root.join(rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::copy(&source, &dest).unwrap();
    dest
}

fn scan_flow(dir: &Path) -> greentic_cards2pack::ir::FlowGroup {
    let config = ScanConfig {
        cards_dir: dir.to_path_buf(),
        group_by: None,
        default_flow: None,
        strict: true,
    };
    let manifest = scan_cards(&config).unwrap();
    manifest.flows.into_iter().next().unwrap()
}

fn extract_generated_block(contents: &str) -> String {
    let start = contents.find("# BEGIN GENERATED (cards2pack)").unwrap();
    let end = contents.find("# END GENERATED (cards2pack)").unwrap();
    contents[start..=end].to_string()
}

#[test]
fn emits_flow_with_routes() {
    let tmp = TempDir::new().unwrap();
    copy_fixture("card-a.json", tmp.path());
    copy_fixture("step-b.json", tmp.path());
    copy_fixture("card-c.json", tmp.path());

    let flow = scan_flow(tmp.path());
    let graph = build_flow_graph(&flow, true).unwrap();
    let flow_path = emit_flow(&graph, tmp.path(), true, None).unwrap().0;
    let contents = fs::read_to_string(flow_path).unwrap();
    let generated = extract_generated_block(&contents);

    assert!(generated.contains("id: demo"));
    assert!(generated.contains("card:"));
    assert!(generated.contains("card_source: asset"));
    assert!(generated.contains("asset_path: assets/cards/card-a.json"));
    assert!(generated.contains("multilingual: true"));
    assert!(generated.contains("step-b"));
    assert!(generated.contains("CARD-C"));
    assert!(generated.contains("call:"));
    assert!(generated.contains("op: render"));
    assert!(generated.contains("metadata: []"));
    assert!(!generated.contains("envelope: {}"));
}

#[test]
fn preserves_developer_content_between_runs() {
    let tmp = TempDir::new().unwrap();
    copy_fixture("card-a.json", tmp.path());
    copy_fixture("step-b.json", tmp.path());
    copy_fixture("card-c.json", tmp.path());

    let flow = scan_flow(tmp.path());
    let graph = build_flow_graph(&flow, true).unwrap();
    let flow_path = emit_flow(&graph, tmp.path(), true, None).unwrap().0;

    let mut contents = fs::read_to_string(&flow_path).unwrap();
    contents.push_str("\n# Developer note\n");
    fs::write(&flow_path, contents).unwrap();

    let flow = scan_flow(tmp.path());
    let graph = build_flow_graph(&flow, true).unwrap();
    let flow_path = emit_flow(&graph, tmp.path(), true, None).unwrap().0;
    let updated = fs::read_to_string(flow_path).unwrap();

    assert!(updated.contains("# Developer note"));
}

#[test]
fn creates_stub_for_missing_target_when_not_strict() {
    let tmp = TempDir::new().unwrap();
    copy_fixture("unresolved.json", tmp.path());

    let flow = scan_flow(tmp.path());
    let graph = build_flow_graph(&flow, false).unwrap();

    assert!(graph.nodes.contains_key("missing-step"));
    assert!(
        graph
            .warnings
            .iter()
            .any(|w| w.message.contains("missing target"))
    );
}

#[test]
fn strict_mode_errors_on_missing_target() {
    let tmp = TempDir::new().unwrap();
    copy_fixture("unresolved.json", tmp.path());

    let flow = scan_flow(tmp.path());
    let result = build_flow_graph(&flow, true);

    assert!(result.is_err());
}
