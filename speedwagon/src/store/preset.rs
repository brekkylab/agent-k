use std::io::Write as _;

use anyhow::Result;
use knowledge_base_examples::{Cached, DocSet as _, FinanceBench};

use super::{FileType, Store};

#[derive(Debug, Clone, clap::ValueEnum, strum::Display)]
#[strum(serialize_all = "kebab-case")]
pub enum PresetKind {
    FinanceBench,
}

pub async fn setup_docset(store: &mut Store, preset: &PresetKind) -> Result<()> {
    match preset {
        PresetKind::FinanceBench => {
            let kb = Cached::new(FinanceBench::new().await?)?;
            let n = kb.num_doc();
            println!("Downloading corpus FinanceBench ({n} documents)...");

            let mut items: Vec<(Vec<u8>, FileType)> = Vec::with_capacity(n);
            for i in 0..n {
                let filename = kb.filename(i).await.unwrap_or_else(|| format!("doc-{i}"));
                println!("[{i}/{n}] {filename}... ");
                let _ = std::io::stdout().flush();
                match kb.read_corpus(i).await {
                    Some(md) => {
                        items.push((md.into().into_bytes(), FileType::MD));
                        println!("ok");
                    }
                    None => println!("skipped"),
                }
            }

            println!("Ingesting corpus FinanceBench...");
            let result = store.ingest_many(items).await?;
            println!(
                "Done. {} documents ingested, {} failed.",
                result.succeeded.len(),
                result.failed.len(),
            );
        }
    }
    Ok(())
}
