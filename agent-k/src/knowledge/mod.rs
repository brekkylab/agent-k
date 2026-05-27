use docling_sys::{convert_docx_to_md, convert_pdf_to_md, convert_pptx_to_md};

pub async fn translate_file(content: Vec<u8>, filetype: &str) -> anyhow::Result<String> {
    match filetype {
        "pdf" => {
            let option = docling_sys::PdfOptions::default();
            convert_pdf_to_md(&content, &option).await
        }
        "docx" => convert_docx_to_md(&content).await,
        "pptx" => convert_pptx_to_md(&content).await,
        "md" => Ok(String::from_utf8(content)?),
        _ => todo!(),
    }
}
