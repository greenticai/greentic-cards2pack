pub mod cli;
pub mod diagnostics;
pub mod emit_flow;
pub mod graph;
pub mod i18n_extract;
pub mod ir;
pub mod qa_integration;
pub mod scan;
pub mod tools;
pub mod translate;
pub mod workspace;

use anyhow::Result;
use cli::{Cli, Commands};

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Generate(args) => workspace::generate(&args),
        Commands::ExtractI18n(args) => {
            let config = i18n_extract::ExtractConfig {
                cards_dir: args.input,
                output: args.output,
                prefix: args.prefix,
                skip_i18n_patterns: !args.include_existing,
            };
            let strings = i18n_extract::extract_from_directory(&config)?;
            i18n_extract::write_bundle(&strings, &config.output)?;
            println!(
                "Extracted {} translatable strings to {}",
                strings.len(),
                config.output.display()
            );
            if args.verbose {
                println!("\n{}", i18n_extract::generate_report(&strings));
            }
            Ok(())
        }
    }
}
