//! Report generation for i18n extraction results.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::ExtractedString;

/// Generate a report of extracted strings.
pub fn generate_report(strings: &[ExtractedString]) -> String {
    let mut report = String::new();
    report.push_str("# I18n Extraction Report\n\n");
    report.push_str(&format!("Total strings extracted: {}\n\n", strings.len()));

    let mut by_file: BTreeMap<PathBuf, Vec<&ExtractedString>> = BTreeMap::new();
    for s in strings {
        by_file.entry(s.source_file.clone()).or_default().push(s);
    }

    for (file, file_strings) in by_file {
        report.push_str(&format!("## {}\n\n", file.display()));
        for s in file_strings {
            report.push_str(&format!("- `{}`: \"{}\"\n", s.key, truncate(&s.value, 50)));
        }
        report.push('\n');
    }

    report
}

/// Truncate a string for display.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}
