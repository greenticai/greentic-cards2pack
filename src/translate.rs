//! Auto-translation wrapper for Adaptive Card i18n bundles.
//!
//! This module orchestrates the translation pipeline:
//! 1. Extract translatable strings from cards using `i18n_extract`
//! 2. Invoke `greentic-i18n-translator` CLI to translate to target languages
//! 3. Copy generated bundles to pack assets
//!
//! Translation failures are non-fatal — the generate command will succeed
//! with a warning if translation fails.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};

const TRANSLATOR_BIN_ENV: &str = "GREENTIC_I18N_TRANSLATOR_BIN";
const TRANSLATOR_DEFAULT_BIN: &str = "greentic-i18n-translator";

use crate::i18n_extract::{self, ExtractConfig};

/// Default target languages when --langs is not specified.
/// Matches the full locale list from `PACK_I18N_LOCALES` (minus "en").
const DEFAULT_LANGUAGES: &[&str] = &[
    "ar", "ar-AE", "ar-DZ", "ar-EG", "ar-IQ", "ar-MA", "ar-SA", "ar-SD", "ar-SY", "ar-TN", "ay",
    "bg", "bn", "cs", "da", "de", "el", "en-GB", "es", "et", "fa", "fi", "fr", "gn", "gu", "hi",
    "hr", "ht", "hu", "id", "it", "ja", "km", "kn", "ko", "lo", "lt", "lv", "ml", "mr", "ms", "my",
    "nah", "ne", "nl", "no", "pa", "pl", "pt", "qu", "ro", "ru", "si", "sk", "sr", "sv", "ta",
    "te", "th", "tl", "tr", "uk", "ur", "vi", "zh",
];

/// Configuration for the auto-translation step.
#[derive(Debug, Clone)]
pub struct TranslateConfig {
    /// Directory containing Adaptive Card JSON files.
    pub cards_dir: PathBuf,
    /// Output directory for i18n bundles (typically pack_dir/assets/i18n).
    pub i18n_output_dir: PathBuf,
    /// Target language codes.
    pub languages: Vec<String>,
    /// Optional glossary file path.
    pub glossary: Option<PathBuf>,
    /// Enable verbose output.
    pub verbose: bool,
    /// Optional extra en.json sources to merge before translating.
    /// Keys from these files are added to en.json after extraction.
    pub merge_en_sources: Vec<PathBuf>,
}

/// Result of the translation step.
#[derive(Debug)]
pub struct TranslateResult {
    /// Number of strings extracted.
    pub strings_extracted: usize,
    /// Languages successfully translated.
    pub languages_translated: Vec<String>,
    /// Languages that failed translation (with error messages).
    pub languages_failed: Vec<(String, String)>,
    /// Path to the English source bundle.
    pub en_bundle_path: PathBuf,
}

/// Ensure the translator binary is available, installing via `cargo binstall` if needed.
fn ensure_translator_available(verbose: bool) -> Result<()> {
    let bin = resolve_translator_bin();

    // If a custom binary path was set via env var, don't try to auto-install.
    if std::env::var(TRANSLATOR_BIN_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
    {
        return Ok(());
    }

    if which::which(&bin).is_ok() {
        return Ok(());
    }

    eprintln!("[translate] greentic-i18n-translator not found, installing via cargo binstall...");

    let mut cmd = Command::new("cargo");
    cmd.arg("binstall")
        .arg("greentic-i18n-translator")
        .arg("--no-confirm");

    if verbose {
        eprintln!("[translate] Running: {:?}", cmd);
    }

    let output = cmd.output().context(
        "failed to run cargo binstall — is cargo-binstall installed? \
         Install it with: cargo install cargo-binstall",
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "cargo binstall greentic-i18n-translator failed:\n{}",
            stderr.trim_end()
        );
    }

    eprintln!("[translate] greentic-i18n-translator installed successfully");
    Ok(())
}

/// Run the auto-translation pipeline.
///
/// This function:
/// 1. Ensures greentic-i18n-translator is installed (auto-installs if missing)
/// 2. Extracts translatable strings from cards
/// 3. Writes the English bundle to `{i18n_output_dir}/en.json`
/// 4. Invokes greentic-i18n-translator for each target language
///
/// Translation failures are captured but do not cause the function to fail.
/// The caller should check `TranslateResult::languages_failed` for any issues.
pub fn run_auto_translate(config: &TranslateConfig) -> Result<TranslateResult> {
    // Auto-install translator if not available
    ensure_translator_available(config.verbose)?;
    // Ensure output directory exists
    std::fs::create_dir_all(&config.i18n_output_dir).with_context(|| {
        format!(
            "failed to create i18n output directory: {:?}",
            config.i18n_output_dir
        )
    })?;

    // Step 1: Extract translatable strings
    let en_bundle_path = config.i18n_output_dir.join("en.json");
    let extract_config = ExtractConfig {
        cards_dir: config.cards_dir.clone(),
        output: en_bundle_path.clone(),
        prefix: "card".to_string(),
        skip_i18n_patterns: true,
    };

    let strings = i18n_extract::extract_from_directory(&extract_config)
        .context("failed to extract i18n strings from cards")?;

    if strings.is_empty() {
        eprintln!("[translate] No translatable strings found in cards");
        return Ok(TranslateResult {
            strings_extracted: 0,
            languages_translated: Vec::new(),
            languages_failed: Vec::new(),
            en_bundle_path,
        });
    }

    // Write English bundle
    i18n_extract::write_bundle(&strings, &en_bundle_path)
        .context("failed to write English i18n bundle")?;

    // Merge additional en.json sources (e.g., existing i18n bundles from source cards).
    let mut merged_count = strings.len();
    for source in &config.merge_en_sources {
        if source.exists()
            && let Ok(raw) = std::fs::read_to_string(source)
            && let Ok(extra) =
                serde_json::from_str::<std::collections::BTreeMap<String, String>>(&raw)
        {
            // Read current en.json, merge, re-write.
            let mut current: std::collections::BTreeMap<String, String> =
                std::fs::read_to_string(&en_bundle_path)
                    .ok()
                    .and_then(|r| serde_json::from_str(&r).ok())
                    .unwrap_or_default();
            for (k, v) in extra {
                current.entry(k).or_insert(v);
            }
            merged_count = current.len();
            let encoded =
                serde_json::to_string_pretty(&current).unwrap_or_else(|_| "{}".to_string());
            let _ = std::fs::write(&en_bundle_path, format!("{encoded}\n"));
        }
    }

    eprintln!(
        "[translate] Extracted {} strings to {}",
        merged_count,
        en_bundle_path.display()
    );

    // Step 2: Translate to each target language (parallel, max 8 concurrent)
    let languages: Vec<String> = if config.languages.is_empty() {
        DEFAULT_LANGUAGES.iter().map(|s| s.to_string()).collect()
    } else {
        config.languages.clone()
    };

    let max_concurrent: usize = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let total = languages.len();
    let done = AtomicUsize::new(0);

    let mut languages_translated = Vec::new();
    let mut languages_failed = Vec::new();

    eprintln!("[translate] Translating to {total} languages...");

    let num_chunks = total.div_ceil(max_concurrent);
    let results: Vec<(String, Result<()>)> = std::thread::scope(|scope| {
        let mut all_results = Vec::new();
        for (chunk_idx, chunk) in languages.chunks(max_concurrent).enumerate() {
            let batch: Vec<_> = chunk.iter().map(|l| l.as_str()).collect();
            eprintln!(
                "[translate] Batch {}/{num_chunks}: {}",
                chunk_idx + 1,
                batch.join(", ")
            );
            let handles: Vec<_> = chunk
                .iter()
                .map(|lang| {
                    let lang = lang.clone();
                    let en_path = en_bundle_path.clone();
                    let done = &done;
                    scope.spawn(move || {
                        let result = translate_to_language(config, &lang, &en_path);
                        let completed = done.fetch_add(1, Ordering::Relaxed) + 1;
                        eprint!("\r[translate] Progress: {completed}/{total}  ");
                        let _ = std::io::stderr().flush();
                        (lang, result)
                    })
                })
                .collect();
            for handle in handles {
                all_results.push(handle.join().unwrap());
            }
            eprintln!();
        }
        all_results
    });

    for (lang, result) in results {
        match result {
            Ok(()) => {
                if config.verbose {
                    eprintln!("[translate] Successfully translated to {lang}");
                }
                languages_translated.push(lang);
            }
            Err(err) => {
                let error_msg = format!("{err:#}");
                // Always print failures — silent failures are confusing.
                eprintln!("[translate] Failed to translate to {lang}: {error_msg}");
                languages_failed.push((lang, error_msg));
            }
        }
    }

    eprintln!(
        "[translate] Done: {} succeeded, {} failed",
        languages_translated.len(),
        languages_failed.len()
    );

    // Write _manifest.json listing all successfully translated locales.
    // Frontends use this to show only languages with actual translations
    // in language selectors.
    write_i18n_manifest(&config.i18n_output_dir, &languages_translated);

    Ok(TranslateResult {
        strings_extracted: strings.len(),
        languages_translated,
        languages_failed,
        en_bundle_path,
    })
}

/// Write `_manifest.json` to the i18n output directory, listing all locale
/// codes that have translation files (including "en").
fn write_i18n_manifest(i18n_dir: &Path, translated_languages: &[String]) {
    let mut locales: Vec<&str> = vec!["en"];
    for lang in translated_languages {
        locales.push(lang.as_str());
    }
    locales.sort();
    locales.dedup();

    let manifest_path = i18n_dir.join("_manifest.json");
    match serde_json::to_string_pretty(&locales) {
        Ok(json) => {
            if let Err(err) = std::fs::write(&manifest_path, json) {
                eprintln!("[translate] warning: failed to write i18n manifest: {err}");
            }
        }
        Err(err) => {
            eprintln!("[translate] warning: failed to serialize i18n manifest: {err}");
        }
    }
}

fn resolve_translator_bin() -> String {
    std::env::var(TRANSLATOR_BIN_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| TRANSLATOR_DEFAULT_BIN.to_string())
}

/// Translate the English bundle to a single target language.
///
/// Each invocation runs in its own temporary working directory to avoid
/// race conditions on the shared `.i18n/translator-state.json` file
/// when multiple translations run in parallel.  The work directory is
/// initialised as a bare git repository so that tools that require a
/// trusted git context (e.g. `codex exec`) don't reject the directory.
///
/// The translator writes output next to the `--en` file, so the
/// translated bundle lands directly in `i18n_output_dir`.
fn translate_to_language(config: &TranslateConfig, lang: &str, en_bundle: &Path) -> Result<()> {
    let bin = resolve_translator_bin();

    // Use a per-language temp directory so parallel translator processes
    // don't clobber each other's state files.
    let work_dir = std::env::temp_dir().join(format!("cards2pack-translate-{lang}"));
    std::fs::create_dir_all(&work_dir)
        .with_context(|| format!("failed to create translator work dir for {lang}"))?;

    // Initialise a git repo in the work dir so codex-cli considers it a
    // trusted directory.  Ignore errors — if git isn't available the
    // translator may still work with --auth-mode api-key.
    if !work_dir.join(".git").exists() {
        let _ = Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(&work_dir)
            .output();
    }

    // The translator writes {lang}.json next to the --en file, so we
    // pass the absolute path to en_bundle.  This ensures the output
    // lands in config.i18n_output_dir regardless of the working dir.
    let en_bundle_abs =
        std::fs::canonicalize(en_bundle).unwrap_or_else(|_| en_bundle.to_path_buf());

    let mut cmd = Command::new(&bin);
    cmd.current_dir(&work_dir)
        .arg("translate")
        .arg("--langs")
        .arg(lang)
        .arg("--en")
        .arg(&en_bundle_abs);

    // Add glossary if provided
    if let Some(glossary) = &config.glossary {
        let glossary_abs = std::fs::canonicalize(glossary).unwrap_or_else(|_| glossary.clone());
        cmd.arg("--glossary").arg(glossary_abs);
    }

    // Add auth mode (default to auto, which tries codex-cli first)
    cmd.arg("--auth-mode").arg("auto");

    if config.verbose {
        eprintln!("[translate] Running: {:?}", cmd);
    }

    let output = cmd
        .output()
        .context("failed to execute greentic-i18n-translator")?;

    // Clean up temp working directory (best-effort)
    let _ = std::fs::remove_dir_all(&work_dir);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        anyhow::bail!(
            "greentic-i18n-translator failed for {}: {}{}",
            lang,
            stderr,
            if !stdout.is_empty() {
                format!("\nstdout: {}", stdout)
            } else {
                String::new()
            }
        );
    }

    Ok(())
}

/// Check if greentic-i18n-translator is available in PATH.
pub fn is_translator_available() -> bool {
    let bin = resolve_translator_bin();
    Command::new(&bin)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Get the list of supported languages from the translator.
///
/// Returns a default list if the translator is not available.
pub fn get_supported_languages() -> Vec<String> {
    // Try to get from translator, fall back to hardcoded list
    DEFAULT_LANGUAGES.iter().map(|s| s.to_string()).collect()
}

/// Format a summary of the translation result for display.
pub fn format_translation_summary(result: &TranslateResult) -> String {
    let mut summary = String::new();

    summary.push_str(&format!(
        "Extracted {} translatable strings\n",
        result.strings_extracted
    ));

    if !result.languages_translated.is_empty() {
        summary.push_str(&format!(
            "Successfully translated to: {}\n",
            result.languages_translated.join(", ")
        ));
    }

    if !result.languages_failed.is_empty() {
        summary.push_str("Translation failures:\n");
        for (lang, error) in &result.languages_failed {
            summary.push_str(&format!("  - {}: {}\n", lang, error));
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_languages() {
        assert!(!DEFAULT_LANGUAGES.is_empty());
        assert!(DEFAULT_LANGUAGES.contains(&"fr"));
        assert!(DEFAULT_LANGUAGES.contains(&"de"));
    }

    #[test]
    fn test_get_supported_languages() {
        let langs = get_supported_languages();
        assert!(!langs.is_empty());
    }

    #[test]
    fn test_format_translation_summary_success() {
        let result = TranslateResult {
            strings_extracted: 42,
            languages_translated: vec!["fr".to_string(), "de".to_string()],
            languages_failed: vec![],
            en_bundle_path: PathBuf::from("en.json"),
        };

        let summary = format_translation_summary(&result);
        assert!(summary.contains("42 translatable strings"));
        assert!(summary.contains("fr, de"));
    }

    #[test]
    fn test_format_translation_summary_with_failures() {
        let result = TranslateResult {
            strings_extracted: 10,
            languages_translated: vec!["fr".to_string()],
            languages_failed: vec![("de".to_string(), "API error".to_string())],
            en_bundle_path: PathBuf::from("en.json"),
        };

        let summary = format_translation_summary(&result);
        assert!(summary.contains("Translation failures"));
        assert!(summary.contains("de: API error"));
    }
}
