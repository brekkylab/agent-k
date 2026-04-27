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
uv sync --extra cpu
```

### Test
To check if the execution environment is properly configured, try running it as follows.

```bash
uv run convert_pdf_to_md.py < input.pdf > output.md
```

## Build binary
Build the binary using pyinstaller, after preparation of execution environment

```bash
./build.sh
```

### Test
```bash
dist/convert_pdf_to_md/convert_pdf_to_md < input.pdf > output.md
```

### Deploy
After the build, you will have the following result `dist/convert_pdf_to_md`.

The executable file `convert_pdf_to_md` and the directory `_internal` inside `dist/convert_pdf_to_md` are essential for executing the built binary.  
Therefore, they must be included together when deploying.

```
.
|____dist
| |____convert_pdf_to_md
| | |____convert_pdf_to_md
| | |_____internal
| | | |____...
```

