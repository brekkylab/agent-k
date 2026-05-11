import sys
from io import BytesIO
import logging
import os

os.environ["HF_HUB_DISABLE_PROGRESS_BARS"] = "1"
logging.getLogger("docling").setLevel(logging.CRITICAL)


def build_converter():
    from docling.datamodel.base_models import InputFormat
    from docling.datamodel.pipeline_options import (
        PdfPipelineOptions,
        TableStructureOptions,
    )
    from docling.document_converter import DocumentConverter, PdfFormatOption

    pipeline_options = PdfPipelineOptions(
        do_ocr=False,
        do_table_structure=True,
        table_structure_options=TableStructureOptions(
            do_cell_matching=True,
            mode="accurate",
        ),
        accelerator_options={"num_threads": 4, "device": "auto"},
        do_picture_classification=False,
        do_picture_description=False,
        do_chart_extraction=False,
        do_code_enrichment=False,
        do_formula_enrichment=False,
        generate_page_images=False,
        generate_picture_images=False,
    )
    return DocumentConverter(
        format_options={
            InputFormat.PDF: PdfFormatOption(pipeline_options=pipeline_options),
        }
    )


def run_docling(stream: BytesIO = None) -> str:
    from docling.document_converter import DocumentStream

    stream = DocumentStream(
        name="target.pdf",
        stream=stream,
    )
    return build_converter().convert(stream).document.export_to_markdown()


def validate_pdf(data: bytes) -> None:
    if not data:
        sys.exit("error: empty input on stdin")
    if b"%PDF-" not in data[:1024]:
        sys.exit(f"error: not a PDF (no %PDF- in first 1024 bytes; head={data[:16]!r})")
    if b"%%EOF" not in data[-1024:]:
        sys.exit("error: PDF EOF marker not found")


def main():
    data = sys.stdin.buffer.read()
    validate_pdf(data)
    markdown = run_docling(BytesIO(data))
    print(markdown)


if __name__ == "__main__":
    import multiprocessing
    multiprocessing.freeze_support()
    main()
