use std::env;

fn main() -> anyhow::Result<()> {
    let path = env::args().nth(1).expect("usage: smoke <path-to-pdf>");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let md = rt.block_on(docling_sys::convert_pdf_file(&path))?;
    println!("--- bundle_dir: {:?}", docling_sys::bundle_dir());
    println!("--- markdown ({} bytes) ---", md.len());
    println!("{md}");
    Ok(())
}
