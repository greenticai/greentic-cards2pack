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
    Generate(GenerateArgs),
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
}

#[derive(ValueEnum, Copy, Clone, Debug, Eq, PartialEq, Serialize)]
pub enum GroupBy {
    Folder,
    FlowField,
}
