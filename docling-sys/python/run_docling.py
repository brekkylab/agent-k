import argparse
import json
import sys
from io import BytesIO
import logging
import os

os.environ["HF_HUB_DISABLE_PROGRESS_BARS"] = "1"
logging.getLogger("docling").setLevel(logging.CRITICAL)


DEFAULT_OPTIONS = {
    "do_ocr": False,
    "do_table_structure": True,
    "do_cell_matching": True,
    "table_structure_mode": "accurate",
    "do_picture_classification": False,
    "do_picture_description": False,
    "do_chart_extraction": False,
    "do_code_enrichment": False,
    "do_formula_enrichment": False,
    "generate_page_images": False,
    "generate_picture_images": False,
    "num_threads": 4,
    "device": "auto",
}


def build_converter(opts: dict):
    from docling.datamodel.base_models import InputFormat
    from docling.datamodel.pipeline_options import (
        PdfPipelineOptions,
        TableStructureOptions,
    )
    from docling.document_converter import DocumentConverter, PdfFormatOption

    pipeline_options = PdfPipelineOptions(
        do_ocr=opts["do_ocr"],
        do_table_structure=opts["do_table_structure"],
        table_structure_options=TableStructureOptions(
            do_cell_matching=opts["do_cell_matching"],
            mode=opts["table_structure_mode"],
        ),
        accelerator_options={
            "num_threads": opts["num_threads"],
            "device": opts["device"],
        },
        do_picture_classification=opts["do_picture_classification"],
        do_picture_description=opts["do_picture_description"],
        do_chart_extraction=opts["do_chart_extraction"],
        do_code_enrichment=opts["do_code_enrichment"],
        do_formula_enrichment=opts["do_formula_enrichment"],
        generate_page_images=opts["generate_page_images"],
        generate_picture_images=opts["generate_picture_images"],
    )
    return DocumentConverter(
        format_options={
            InputFormat.PDF: PdfFormatOption(pipeline_options=pipeline_options),
        }
    )


def run_docling(stream: BytesIO, opts: dict) -> str:
    from docling.document_converter import DocumentStream

    stream = DocumentStream(
        name="target.pdf",
        stream=stream,
    )
    return build_converter(opts).convert(stream).document.export_to_markdown()


def validate_pdf(data: bytes) -> None:
    if not data:
        sys.exit("error: empty input on stdin")
    if b"%PDF-" not in data[:1024]:
        sys.exit(f"error: not a PDF (no %PDF- in first 1024 bytes; head={data[:16]!r})")
    if b"%%EOF" not in data[-1024:]:
        sys.exit("error: PDF EOF marker not found")


def parse_options(raw: str | None) -> dict:
    if not raw:
        return dict(DEFAULT_OPTIONS)
    try:
        provided = json.loads(raw)
    except json.JSONDecodeError as e:
        sys.exit(f"error: --options is not valid JSON: {e}")
    if not isinstance(provided, dict):
        sys.exit("error: --options must be a JSON object")
    merged = dict(DEFAULT_OPTIONS)
    unknown = set(provided) - set(DEFAULT_OPTIONS)
    if unknown:
        sys.exit(f"error: unknown option keys: {sorted(unknown)}")
    merged.update(provided)
    return merged


def main():
    parser = argparse.ArgumentParser(description="Convert a PDF on stdin to Markdown on stdout.")
    parser.add_argument(
        "--options",
        help="JSON object with pipeline options. Missing keys fall back to defaults.",
        default=None,
    )
    args = parser.parse_args()
    opts = parse_options(args.options)

    data = sys.stdin.buffer.read()
    validate_pdf(data)
    markdown = run_docling(BytesIO(data), opts)
    print(markdown)


if __name__ == "__main__":
    import multiprocessing
    multiprocessing.freeze_support()
    main()
