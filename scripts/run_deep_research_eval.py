#!/usr/bin/env python3
"""Run all 10 deep-research test cases sequentially, persist each run's
artifacts, and produce a summary report.

Outputs land under `eval-runs/deep-research-<timestamp>/`:
    runs/<case_no>/                # the agent's artifacts/ directory
    runs/<case_no>/stdout.txt
    runs/<case_no>/stderr.txt
    results.jsonl                  # one JSON line per case
    results.csv                    # flattened metrics for spreadsheet use
    report.md                      # human-readable summary
    report.html                    # same summary rendered as HTML

Usage:
    python3 scripts/run_deep_research_eval.py [--model openai|claude|gemini] [--timeout SEC]
"""
from __future__ import annotations

import argparse
import csv
import datetime as dt
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BINARY = REPO_ROOT / "target" / "release" / "test_case"
ARTIFACT_DIR = REPO_ROOT / "artifacts"
EVAL_BASE = REPO_ROOT / "eval-runs"

# 10 cases, paired EN/KO per topic, matching cases/deep_research.rs.
CASES = [
    (0, "space/astronomy", "en", "medium"),
    (1, "space/astronomy", "ko", "medium"),
    (2, "history", "en", "medium"),
    (3, "history", "ko", "medium"),
    (4, "climate/energy", "en", "medium"),
    (5, "climate/energy", "ko", "medium"),
    (6, "philosophy", "en", "hard"),
    (7, "philosophy", "ko", "hard"),
    (8, "cuisine/food", "en", "hard"),
    (9, "cuisine/food", "ko", "hard"),
]


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser()
    p.add_argument("--model", default="openai", choices=["openai", "claude", "gemini"])
    p.add_argument("--timeout", type=int, default=600, help="per-case timeout in seconds")
    p.add_argument("--only", type=str, default=None, help="comma-separated case numbers")
    p.add_argument("--local", action="store_true",
                   help="use RunEnv::local() (skip sandbox; ~3-4× faster, evaluation only)")
    return p.parse_args()


def run_one(case_no: int, model: str, timeout: int, run_dir: Path, local: bool = False) -> dict:
    """Run one test_case invocation. Returns a dict of metrics."""
    if ARTIFACT_DIR.exists():
        shutil.rmtree(ARTIFACT_DIR)

    case_dir = run_dir / f"case_{case_no:02d}"
    case_dir.mkdir(parents=True, exist_ok=True)

    cmd = [str(BINARY), "deep-research", str(case_no), "--model", model]
    if local:
        cmd.append("--local")

    start = time.monotonic()
    timed_out = False
    try:
        proc = subprocess.run(
            cmd,
            cwd=str(REPO_ROOT),
            capture_output=True,
            text=True,
            timeout=timeout,
            input="",
        )
        stdout, stderr, exit_code = proc.stdout, proc.stderr, proc.returncode
    except subprocess.TimeoutExpired as e:
        timed_out = True
        stdout = (e.stdout.decode("utf-8", "replace") if e.stdout else "")
        stderr = (e.stderr.decode("utf-8", "replace") if e.stderr else "")
        exit_code = -1
    elapsed = time.monotonic() - start

    (case_dir / "stdout.txt").write_text(stdout)
    (case_dir / "stderr.txt").write_text(stderr)

    artifacts_out = case_dir / "artifacts"
    if ARTIFACT_DIR.exists():
        shutil.copytree(ARTIFACT_DIR, artifacts_out, dirs_exist_ok=True)
    # Allow timed_out to be inspected by caller via proc.stdout regex match later.
    proc_stdout = stdout

    # Count tool calls from stdout — the agent prints `[deep-research] tool: NAME ARGS_JSON`.
    tool_lines = re.findall(r"\[deep-research\] tool: (\S+)\s+(\{.*?\})\s*$", proc_stdout, re.M)
    counts: dict[str, int] = {}
    for name, _ in tool_lines:
        counts[name] = counts.get(name, 0) + 1

    report_path = artifacts_out / "report.md"
    citations_path = artifacts_out / "citations.json"
    report_bytes = report_path.stat().st_size if report_path.exists() else 0
    n_citations = 0
    n_marker_unique = 0
    if citations_path.exists():
        try:
            cites = json.loads(citations_path.read_text())
            if isinstance(cites, dict):
                n_citations = len(cites)
        except Exception:
            pass
    if report_path.exists():
        body = report_path.read_text()
        n_marker_unique = len(set(re.findall(r"\[\^(\d+)\]", body)))

    return {
        "case_no": case_no,
        "exit_code": exit_code,
        "timed_out": timed_out,
        "wall_seconds": round(elapsed, 1),
        "report_bytes": report_bytes,
        "n_citations": n_citations,
        "n_unique_citation_markers": n_marker_unique,
        "api_search_calls": counts.get("api_search", 0),
        "web_fetch_calls": counts.get("web_fetch", 0),
        "shell_calls": counts.get("shell", 0),
        "total_tool_calls": sum(counts.values()),
    }


def render_markdown_report(rows: list[dict], meta: dict, out: Path) -> None:
    lines: list[str] = []
    lines.append(f"# Deep-research eval — {meta['timestamp']}")
    lines.append("")
    lines.append(f"- Model: `{meta['model']}`")
    lines.append(f"- Per-case timeout: {meta['timeout']}s")
    lines.append(f"- Cases run: {len(rows)} / 10")
    lines.append(f"- ailoy rev: `{meta.get('ailoy_rev', 'unknown')}`")
    lines.append("")
    lines.append("## Per-case results")
    lines.append("")
    lines.append(
        "| # | domain | lang | difficulty | exit | wall (s) | report bytes | citations | api_search | web_fetch | total tools |"
    )
    lines.append("|---|---|---|---|---|---|---|---|---|---|---|")
    for r in rows:
        case_no = r["case_no"]
        info = next(c for c in CASES if c[0] == case_no)
        domain, lang, diff = info[1], info[2], info[3]
        lines.append(
            f"| {case_no} | {domain} | {lang} | {diff} | {r['exit_code']} | "
            f"{r['wall_seconds']} | {r['report_bytes']} | {r['n_citations']} | "
            f"{r['api_search_calls']} | {r['web_fetch_calls']} | {r['total_tool_calls']} |"
        )
    lines.append("")
    completed = [r for r in rows if r["exit_code"] == 0 and r["report_bytes"] > 0]
    lines.append(f"- Completed (exit=0 with non-empty report): {len(completed)} / {len(rows)}")
    if completed:
        avg_wall = sum(r["wall_seconds"] for r in completed) / len(completed)
        avg_cites = sum(r["n_citations"] for r in completed) / len(completed)
        avg_search = sum(r["api_search_calls"] for r in completed) / len(completed)
        avg_fetch = sum(r["web_fetch_calls"] for r in completed) / len(completed)
        lines.append(f"- Avg wall (completed only): {avg_wall:.1f}s")
        lines.append(f"- Avg citations: {avg_cites:.1f}")
        lines.append(f"- Avg api_search calls: {avg_search:.1f}")
        lines.append(f"- Avg web_fetch calls: {avg_fetch:.1f}")
    out.write_text("\n".join(lines))


def render_html_report(rows: list[dict], meta: dict, out: Path) -> None:
    import html

    table_rows = ""
    for r in rows:
        info = next(c for c in CASES if c[0] == r["case_no"])
        table_rows += (
            f"<tr><td>{r['case_no']}</td><td>{html.escape(info[1])}</td>"
            f"<td>{info[2]}</td><td>{info[3]}</td>"
            f"<td>{r['exit_code']}</td><td>{r['wall_seconds']}</td>"
            f"<td>{r['report_bytes']}</td><td>{r['n_citations']}</td>"
            f"<td>{r['api_search_calls']}</td><td>{r['web_fetch_calls']}</td>"
            f"<td>{r['total_tool_calls']}</td></tr>\n"
        )
    completed = [r for r in rows if r["exit_code"] == 0 and r["report_bytes"] > 0]
    summary_extra = ""
    if completed:
        avg_wall = sum(r["wall_seconds"] for r in completed) / len(completed)
        summary_extra = (
            f"<p>Completed: {len(completed)} / {len(rows)}. "
            f"Avg wall on completed: {avg_wall:.1f}s.</p>"
        )
    body = f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>Deep-research eval — {meta['timestamp']}</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, Segoe UI, Roboto, sans-serif;
        max-width: 1100px; margin: 2rem auto; padding: 0 1rem; }}
table {{ border-collapse: collapse; width: 100%; }}
th, td {{ border: 1px solid #d0d7de; padding: 6px 10px; text-align: right; font-variant-numeric: tabular-nums; }}
th:nth-child(2), td:nth-child(2),
th:nth-child(3), td:nth-child(3),
th:nth-child(4), td:nth-child(4) {{ text-align: left; }}
th {{ background: #f6f8fa; }}
caption {{ font-weight: 600; padding: 8px 0; }}
</style></head>
<body>
<h1>Deep-research eval — {meta['timestamp']}</h1>
<p>Model: <code>{meta['model']}</code> · Timeout: {meta['timeout']}s · ailoy rev: <code>{meta.get('ailoy_rev','unknown')}</code></p>
{summary_extra}
<table>
<caption>Per-case metrics</caption>
<thead>
<tr><th>#</th><th>domain</th><th>lang</th><th>difficulty</th>
<th>exit</th><th>wall (s)</th><th>report bytes</th><th>citations</th>
<th>api_search</th><th>web_fetch</th><th>total tools</th></tr>
</thead>
<tbody>
{table_rows}
</tbody></table>
</body></html>"""
    out.write_text(body)


def main() -> int:
    args = parse_args()
    if not BINARY.exists():
        print(f"binary not built: {BINARY}", file=sys.stderr)
        return 2

    selected = (
        [int(s) for s in args.only.split(",")] if args.only
        else [c[0] for c in CASES]
    )

    ts = dt.datetime.now().strftime("%Y%m%dT%H%M%S")
    run_root = EVAL_BASE / f"deep-research-{args.model}-{ts}"
    run_root.mkdir(parents=True, exist_ok=True)

    ailoy_rev = "unknown"
    cargo_toml = REPO_ROOT / "Cargo.toml"
    if cargo_toml.exists():
        m = re.search(r'rev\s*=\s*"([0-9a-f]+)"', cargo_toml.read_text())
        if m:
            ailoy_rev = m.group(1)[:12]

    meta = {
        "timestamp": ts,
        "model": args.model,
        "timeout": args.timeout,
        "ailoy_rev": ailoy_rev,
    }
    (run_root / "meta.json").write_text(json.dumps(meta, indent=2))

    results: list[dict] = []
    jsonl = (run_root / "results.jsonl").open("w")
    for case_no in selected:
        info = next(c for c in CASES if c[0] == case_no)
        print(
            f"[{case_no}/{max(selected)}] domain={info[1]} lang={info[2]} diff={info[3]} ...",
            flush=True,
        )
        row = run_one(case_no, args.model, args.timeout, run_root, local=args.local)
        flag = " TIMEOUT" if row.get("timed_out") else ""
        print(
            f"  -> exit={row['exit_code']}{flag} wall={row['wall_seconds']}s "
            f"report={row['report_bytes']}B cites={row['n_citations']} "
            f"api_search={row['api_search_calls']} web_fetch={row['web_fetch_calls']}",
            flush=True,
        )
        results.append(row)
        jsonl.write(json.dumps(row) + "\n")
        jsonl.flush()
    jsonl.close()

    fields = list(results[0].keys()) if results else []
    with (run_root / "results.csv").open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        for r in results:
            w.writerow(r)

    render_markdown_report(results, meta, run_root / "report.md")
    render_html_report(results, meta, run_root / "report.html")
    print(f"\nrun root: {run_root}")
    print(f"  report.md  : {run_root / 'report.md'}")
    print(f"  report.html: {run_root / 'report.html'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
