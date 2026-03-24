use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

#[derive(Parser)]
#[command(name = "greentic-cards2pack")]
#[command(about = "Generate Greentic pack workspace from Adaptive Cards", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Generate pack workspace from Adaptive Cards.
    Generate(GenerateArgs),
    /// Extract translatable strings from Adaptive Cards for i18n.
    ///
    /// Scans cards for text fields (text, title, placeholder, etc.) and
    /// generates an initial English translation bundle.
    #[command(name = "extract-i18n")]
    ExtractI18n(ExtractI18nArgs),
}

/// Arguments for the extract-i18n command.
#[derive(Args, Debug)]
pub struct ExtractI18nArgs {
    /// Directory containing Adaptive Card JSON files.
    #[arg(short, long)]
    pub input: PathBuf,

    /// Output JSON file path (e.g., i18n/en.json).
    #[arg(short, long, default_value = "i18n/en.json")]
    pub output: PathBuf,

    /// Key prefix for generated keys (e.g., "card").
    #[arg(long, default_value = "card")]
    pub prefix: String,

    /// Include strings that already contain $t() patterns.
    #[arg(long)]
    pub include_existing: bool,

    /// Print detailed extraction report.
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Args, Debug)]
pub struct GenerateArgs {
    /// Directory of Adaptive Card JSON files.
    #[arg(long)]
    pub cards: PathBuf,
    /// Output workspace directory.
    #[arg(long)]
    pub out: PathBuf,
    /// Pack name and dist artifact name.
    #[arg(long)]
    pub name: String,
    /// Path to greentic-pack binary.
    #[arg(long)]
    pub greentic_pack_bin: Option<PathBuf>,
    /// Grouping strategy for flows (stored only).
    #[arg(long, value_enum)]
    pub group_by: Option<GroupBy>,
    /// Default flow name (stored only).
    #[arg(long)]
    pub default_flow: Option<String>,
    /// Strict mode (stored only).
    #[arg(long)]
    pub strict: bool,
    /// Print greentic-pack command and output.
    #[arg(long)]
    pub verbose: bool,
    /// Prompt-based routing (requires prompt2flow component).
    #[arg(long)]
    pub prompt: bool,
    /// Disable automatic card text extraction/rewriting to i18n markers.
    #[arg(long = "no-auto-i18n")]
    pub no_auto_i18n: bool,
    /// Answers JSON produced by greentic-qa (requires --prompt).
    #[arg(long = "prompt-json")]
    pub prompt_json: Option<PathBuf>,
    /// Override prompt limits via JSON string or file (requires --prompt).
    #[arg(long = "prompt-limits")]
    pub prompt_limits: Option<String>,

    // ---------------------------------------------------------------------------
    // Auto-translation options
    // ---------------------------------------------------------------------------
    /// Enable automatic translation of Adaptive Card strings.
    ///
    /// When enabled, extracts translatable strings from cards, generates
    /// locale-specific i18n bundles, and includes them in the pack assets.
    /// Requires greentic-i18n-translator to be available in PATH.
    #[arg(long)]
    pub auto_translate: bool,

    /// Comma-separated list of target language codes (e.g., "fr,de,ja,es").
    ///
    /// Specifies which languages to translate to. Each language generates
    /// a separate i18n bundle (e.g., fr.json, de.json). If not specified
    /// with --auto-translate, defaults to common languages.
    #[arg(long, value_delimiter = ',')]
    pub langs: Option<Vec<String>>,

    /// Path to a glossary JSON file for translation consistency.
    ///
    /// The glossary contains term mappings to ensure consistent translations
    /// of domain-specific terminology across all target languages.
    #[arg(long)]
    pub glossary: Option<PathBuf>,
}

#[derive(ValueEnum, Copy, Clone, Debug, Eq, PartialEq, Serialize)]
pub enum GroupBy {
    Folder,
    FlowField,
}
