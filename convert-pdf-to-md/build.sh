rm -f run.spec
.venv/bin/pyinstaller run.py \
    --onedir \
    --noconfirm \
    --recursive-copy-metadata=docling \
    --collect-all=docling \
    --collect-all=docling_core \
    --collect-all=docling_ibm_models \
    --collect-all=docling_parse \
    --exclude-module=hf_xet \
    --exclude-module=faker \
    --exclude-module=tree_sitter \
    --exclude-module=tree_sitter_typescript \
    --exclude-module=tree_sitter_c \
    --exclude-module=tree_sitter_javascript \
    --exclude-module=tree_sitter_python
