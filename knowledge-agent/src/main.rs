use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use clap::Parser;
use knowledge_agent::{
    AppConfig, IndexSettings, SearchIndex, build_agent, build_index_multi, run_tui,
};

#[derive(Parser)]
#[command(
    name = "knowledge-agent",
    about = "Interactive knowledge-base agent with TUI"
)]
struct Cli {
    /// Directories to index and search (defaults to current directory)
    #[arg(long, num_args = 1..)]
    target_paths: Option<Vec<PathBuf>>,

    /// Force re-indexing even if index appears up to date
    #[arg(long)]
    reindex: bool,

    /// Build index only, then exit (no TUI)
    #[arg(long)]
    index_only: bool,

    /// Directory to write the index into (defaults to CWD/.index)
    #[arg(long)]
    index_dir: Option<PathBuf>,

    /// Path to JSON config file
    #[arg(long)]
    config: Option<PathBuf>,

    /// Use NoMergePolicy with per-document commits (1 doc = 1 segment)
    #[arg(long)]
    no_merge: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if Path::new(".env").exists() {
        dotenvy::dotenv()?;
    }

    // 1. Load config (file or default)
    let app_config = if let Some(config_path) = &cli.config {
        AppConfig::from_file(config_path)?
    } else {
        AppConfig::default()
    };

    // 2. Determine target paths
    let target_dirs: Vec<PathBuf> = cli
        .target_paths
        .unwrap_or_else(|| vec![std::env::current_dir().expect("cannot get CWD")]);

    // 3. Index directory
    let index_dir = cli.index_dir.unwrap_or_else(|| {
        std::env::current_dir()
            .expect("cannot get CWD")
            .join(".index")
    });

    // 4. Build index
    let settings = IndexSettings {
        schema_version: 1,
        no_merge: cli.no_merge,
    };
    println!("=== Indexing ===");
    let report = build_index_multi(&index_dir, &target_dirs, &settings, cli.reindex)?;

    if cli.index_only {
        println!("\n=== Index Report ===");
        println!(
            "Files: {} total, {} success, {} failed, {} skipped",
            report.total_files, report.success_count, report.fail_count, report.skipped_count
        );
        println!(
            "Timing: index={:.3}s, commit={:.2}s, total={:.1}s",
            report.total_index_secs, report.commit_secs, report.total_duration_secs
        );
        let output_path = "index_report.json";
        std::fs::write(output_path, serde_json::to_string_pretty(&report)?)?;
        println!("Report saved to {output_path}");
        return Ok(());
    }

    // 5. Open search index
    let search_index = Arc::new(SearchIndex::open(&index_dir)?);
    println!(
        "Index opened: {} files indexed",
        search_index.indexed_filepaths()?.len()
    );

    // 6. Build agent
    let agent = build_agent(
        &app_config.agent,
        &app_config.tool,
        &search_index,
        target_dirs.clone(),
    )
    .await?;

    // 7. Launch TUI
    run_tui(app_config, agent, search_index, target_dirs).await?;

    Ok(())
}
