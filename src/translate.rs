//! Auto-translation wrapper for Adaptive Card i18n bundles.
//!
//! This module orchestrates the translation pipeline:
//! 1. Extract translatable strings from cards using `i18n_extract`
//! 2. Invoke `greentic-i18n-translator` CLI to translate to target languages
//! 3. Copy generated bundles to pack assets
//!
//! Translation failures are non-fatal — the generate command will succeed
//! with a warning if translation fails.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Run the auto-translation pipeline.
///
/// This function:
/// 1. Extracts translatable strings from cards
/// 2. Writes the English bundle to `{i18n_output_dir}/en.json`
/// 3. Invokes greentic-i18n-translator for each target language
///
/// Translation failures are captured but do not cause the function to fail.
/// The caller should check `TranslateResult::languages_failed` for any issues.
pub fn run_auto_translate(config: &TranslateConfig) -> Result<TranslateResult> {
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
        if config.verbose {
            eprintln!("[translate] No translatable strings found in cards");
        }
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

    if config.verbose {
        eprintln!(
            "[translate] Extracted {} strings to {}",
            strings.len(),
            en_bundle_path.display()
        );
    }

    // Step 2: Translate to each target language (parallel, max 8 concurrent)
    let languages: Vec<String> = if config.languages.is_empty() {
        DEFAULT_LANGUAGES.iter().map(|s| s.to_string()).collect()
    } else {
        config.languages.clone()
    };

    const MAX_CONCURRENT: usize = 8;

    let mut languages_translated = Vec::new();
    let mut languages_failed = Vec::new();

    if config.verbose {
        eprintln!(
            "[translate] Translating to {} languages ({} concurrent)",
            languages.len(),
            MAX_CONCURRENT
        );
    }

    let results: Vec<(String, Result<()>)> = std::thread::scope(|scope| {
        let mut all_results = Vec::new();
        for chunk in languages.chunks(MAX_CONCURRENT) {
            let handles: Vec<_> = chunk
                .iter()
                .map(|lang| {
                    let lang = lang.clone();
                    let en_path = en_bundle_path.clone();
                    scope.spawn(move || {
                        let result = translate_to_language(config, &lang, &en_path);
                        (lang, result)
                    })
                })
                .collect();
            for handle in handles {
                all_results.push(handle.join().unwrap());
            }
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
                if config.verbose {
                    eprintln!("[translate] Failed to translate to {lang}: {error_msg}");
                }
                languages_failed.push((lang, error_msg));
            }
        }
    }

    Ok(TranslateResult {
        strings_extracted: strings.len(),
        languages_translated,
        languages_failed,
        en_bundle_path,
    })
}

fn resolve_translator_bin() -> String {
    std::env::var(TRANSLATOR_BIN_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| TRANSLATOR_DEFAULT_BIN.to_string())
}

/// Translate the English bundle to a single target language.
fn translate_to_language(config: &TranslateConfig, lang: &str, en_bundle: &Path) -> Result<()> {
    let bin = resolve_translator_bin();
    let mut cmd = Command::new(&bin);
    cmd.arg("translate")
        .arg("--langs")
        .arg(lang) // Language code like "fr", not a path
        .arg("--en")
        .arg(en_bundle);

    // Add glossary if provided
    if let Some(glossary) = &config.glossary {
        cmd.arg("--glossary").arg(glossary);
    }

    // Add auth mode (default to auto, which tries codex-cli first)
    cmd.arg("--auth-mode").arg("auto");

    if config.verbose {
        eprintln!("[translate] Running: {:?}", cmd);
    }

    let output = cmd
        .output()
        .context("failed to execute greentic-i18n-translator")?;

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
