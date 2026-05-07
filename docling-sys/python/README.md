# Convert PDF to Markdown
This program takes a single PDF file as input from `stdin` and outputs the converted markdown to `stdout`.

## Prepare execution environment
First, configure the execution environment using uv.
To install pyinstaller along with it, add `--dev`.

```bash
uv sync --dev
```

### CPU-Only version
To install a CPU-only torch wheel excluding NVIDIA dependencies for environments without a GPU or for reducing PyInstaller binary size, use the following command.

```bash
uv sync --dev --extra cpu
```

When building via the parent `docling-sys` Rust crate, enable the `cpu` cargo feature to use this CPU-only wheel automatically:

```bash
cargo build -p docling-sys --features cpu
```

### Test
To check if the execution environment is properly configured, try running it as follows.

```bash
uv run run_docling.py < input.pdf > output.md
```

## Build binary
Build the binary using pyinstaller, after preparation of execution environment

```bash
./build.sh
```

### Test
```bash
dist/run_docling/run_docling < input.pdf > output.md
```

### Deploy
After the build, you will have the following result `dist/run_docling`.

The executable file `run_docling` and the directory `_internal` inside `dist/run_docling` are essential for executing the built binary.  
Therefore, they must be included together when deploying.

```
.
|____dist
| |____run_docling
| | |____run_docling
| | |_____internal
| | | |____...
```

