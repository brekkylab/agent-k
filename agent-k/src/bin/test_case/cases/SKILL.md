---
name: xlsx
description: Create or edit any Excel .xlsx file — single sheet or multi-sheet, with formulas, charts, conditional formatting, named ranges, print setup, currency/date formatting, etc. — so it opens cleanly in Excel (no "file corrupted"/recovery dialog, no #REF!/#NAME?/#DIV/0! errors). Use this for ANY task that produces or modifies an .xlsx file.
---

# XLSX Creation Skill

Python `.xlsx` Excel generator (openpyxl, Python 3.10+; LibreOffice
headless for the verify/read/autofit helpers). Output must open in Excel
with zero formula errors (`#REF!` / `#DIV/0!` / `#VALUE!` / `#N/A` /
`#NAME?`) and no "file corrupted" or recovery dialogs.

---

# Skill API Reference

Full implementation is in [`xlsx_skill.py`](xlsx_skill.py). **Import it and
call it as documented — do NOT open or read the source.** This reference
already covers every public signature, the auto-config behavior
(`configure_sheet`: freeze/filter/widths/heights), the visual-width column
sizing, and the sheet-name quoting rules. Reading the ~500-line source
only burns tokens and adds nothing — **only** open it as a last resort if
something genuinely fails and this reference doesn't explain it.

```python id="skill_setup"
import sys
sys.path.insert(0, "artifacts")  # so `xlsx_skill` is importable

from xlsx_skill import (
    XLSXReportSkill, Formula, verify_formulas, read_computed, autofit_columns
)
```

## `XLSXReportSkill`

Stateful builder for a workbook. One instance per output `.xlsx`.

| Member | Signature / Type | Purpose |
|---|---|---|
| `__init__()` | `() -> None` | Creates an empty workbook (no default sheet). |
| `add_sheet(name, title, headers, rows)` | `(str, str, list[str], list[list]) -> Worksheet` | Adds a sheet, renders layout, applies styling and config. |
| `add_defined_name(name, sheet_name, range_ref)` | `(str, str, str) -> str` | Register a named range; sheet name is auto-quoted for CJK/spaces (avoids Excel "file corrupted" dialog). |
| `quote_sheet_name(sheet_name)` *(classmethod)* | `(str) -> str` | Returns `'Sheet'` if Excel needs the name quoted, else as-is. |
| `quote_sheet_ref(sheet_name, range_ref)` *(classmethod)* | `(str, str) -> str` | Builds `'Sheet'!$A$1:$A$10` with auto-quoting. |
| `save(path)` | `(str) -> str` | Saves the workbook to `path`; returns the path. |

`rows` cells may be primitives (`int`, `str`, `date`, `datetime`) or
formula strings starting with `=`. ISO date strings (`YYYY-MM-DD`) are
auto-converted to real date values. Row layout (title r1 / spacer r2 /
header r3 / data r4+) is shown in the **Row heights** table below.

### Final steps after `save()` — always run these

openpyxl writes formula *strings* and *guesses* formula-column widths, so
two things stay broken until LibreOffice computes the sheet. **After
`save()`, always run both** (do not skip these or hand-roll `libreoffice`
yourself — these helpers already do it right):

**1. `autofit_columns(path)`** — resize every column from the *real
computed value*. Without it, formula/currency columns are guessed too
narrow and show `######` in Excel. Number-format aware (`₩2,989,000`) and
east-asian-width aware for CJK that a formula returns. **Required whenever
any column holds formulas or currency.**

**2. `verify_formulas(path)`** — recalculate and return error cells
(`#NAME?` from a missing `_xlfn.` prefix, `#REF!`, `#DIV/0!`, ...). An
empty list means every formula computed cleanly.

```python id="skill_finalize"
report.save("artifacts/report.xlsx")
autofit_columns("artifacts/report.xlsx")            # fix ###### on formula/currency columns
errors = verify_formulas("artifacts/report.xlsx")   # [] = no formula errors
if errors:
    print(errors)   # [('Grades','I4','#NAME?'), ...] → fix and re-save
```

**Optional — value sanity check.** `read_computed(path, cells=None)`
returns computed values so you can compare against expectations:

```python id="skill_readcomputed"
vals = read_computed("artifacts/report.xlsx",
                     {"Data": ["L4"], "Summary": ["B16", "B27"]})
assert vals["Data"]["L4"] == 1466000 + 36650 + 293200   # supply + fee + VAT
assert vals["Summary"]["B16"] == vals["Summary"]["B27"]  # monthly total == quarterly total
```

Neither helper catches Excel-only structural issues (unquoted CJK sheet
names, orphan panes) — LibreOffice opens those without complaint, so those
still rely on the helpers below.

### ⚠️ Sheet names with CJK / spaces

Cross-sheet refs to non-ASCII sheet names **must** be single-quoted in
Excel, otherwise Excel marks the file as corrupted on open.

```text
❌ =SUM(원본데이터!$K$4:$K$203)        # Excel: "The file is corrupted"
✅ =SUM('원본데이터'!$K$4:$K$203)
```
→ Use `report.add_defined_name(...)` or `report.quote_sheet_ref(sheet, range)`.

### Instance attributes

`report.wb` — underlying `openpyxl.Workbook`. Use it for direct openpyxl
operations not exposed by the skill (`report.wb["Summary"]`, conditional
formatting, etc.). Prefer `report.save(path)` for saving.

### Style constants (class attributes)

| Constant | Applied to |
|---|---|
| `TITLE_FILL` / `WHITE_FONT` | Title row 1 (fill + font) |
| `HEADER_FILL` / `HEADER_FONT` | Header row 3 (fill + font) |
| `NORMAL_FONT` | Data rows (body text) |
| `ROW_FILL` | Zebra fill on even data rows |
| `BORDER` | Default cell border |

Default values are sensible (dark title, accent header, light zebra,
thin border). Applied automatically by `apply_style` — no need to
reapply. To diverge — KPI cards in a different color, accent rows,
custom theme, etc. — pick your own values and override after `add_sheet`:

```python
from openpyxl.styles import PatternFill, Font
ws["B3"].fill = PatternFill(start_color="4472C4", end_color="4472C4", fill_type="solid")
ws["B3"].font = Font(bold=True, size=14, color="FFFFFF")
```

### Row heights (applied by `configure_sheet`)

| Constant | Pt | Applied to |
|---|---|---|
| `ROW_HEIGHT_TITLE` | 28 | Row 1 (title) |
| `ROW_HEIGHT_SPACER` | 10 | Row 2 (spacer) |
| `ROW_HEIGHT_HEADER` | 22 | Row 3 (header) |
| `ROW_HEIGHT_DATA` | 20 | Rows 4+ (data) |
| `ROW_HEIGHT_SUMMARY` | 22 | Reuse for your summary/total rows |

Override after `add_sheet` only when content truly needs more (multi-line
wrapped text, KPI cards, etc.):

```python
ws.row_dimensions[4].height = 40   # taller KPI card row
```

### Auto-configured by `configure_sheet`

* `freeze_panes = "A4"`
* `auto_filter.ref = "A3:{last_col}{max_row}"`
* Column widths (visual-width aware, see below)
* Row heights per the table above

Override after `add_sheet` when the layout differs (dashboard sheets,
multi-block layouts, etc.):

```python
ws.freeze_panes = None        # dashboard / KPI-only sheet
ws.freeze_panes = "A3"        # fix only the title row
ws.auto_filter.ref = None     # no filter
```

### Column width policy (auto)

Visual width via `unicodedata.east_asian_width()` (CJK / Hangul /
full-width Latin = 2). Only the **main data block** (row 3 → first
all-blank spacer row) is measured; title row and summary block below
are excluded. Formula cells can't be evaluated here, so they're sized to
`COLUMN_WIDTH_FORMULA_DEFAULT` (a guess) — see `autofit_columns` below to
fix that.

Width = `clamp(visual + PADDING, MIN, MAX)`.

| Constant | Default |
|---|---|
| `COLUMN_WIDTH_MIN` | 10 |
| `COLUMN_WIDTH_MAX` | 40 |
| `COLUMN_WIDTH_PADDING` | 4 |
| `COLUMN_WIDTH_FORMULA_DEFAULT` | 8 |

For auto-exclude to work, leave **one blank row** between data and
summary block. Override after `add_sheet`:

```python id="skill_colwidth_override"
ws.column_dimensions["C"].width = 18           # fixed width
ws.auto_filter.ref = f"A3:J{DATA_END}"         # exclude summary rows
```

This auto-width only *guesses* formula columns (→ `######`). The real fix
is `autofit_columns(path)` after `save()` — see **Final steps** above.

### ⚠️ Excel Table + named ranges + autoFilter

Adding an Excel Table (`openpyxl.worksheet.table.Table`) to a sheet that
`add_sheet` already configured causes Excel's **"The file is corrupted"**
dialog from two collisions:

1. **Table column names ↔ workbook named ranges** with the same name.
2. **Triple autoFilter** on the same range.

```text
✅ When using a Table:
   - prefix named ranges: DefinedName("R_Region", ...)
   - clear the sheet-level filter: ws.auto_filter.ref = None
```

### Charts — how to add one

The anchor is a **plain cell string** (`"B8"`). `add_chart` builds the
`OneCellAnchor` for you — never construct `OneCellAnchor` / `AnchorMarker`
/ `XDRPositiveSize2D` by hand.

```python id="skill_chart"
from openpyxl.chart import BarChart, LineChart, PieChart, Reference

chart = BarChart()                 # or LineChart() / PieChart()
chart.type = "col"                 # BarChart only: "col"=vertical, "bar"=horizontal
chart.title = "Monthly Sales"
# data = value column(s); include the header row so the series gets a name
chart.add_data(Reference(ws, min_col=2, min_row=3, max_row=15), titles_from_data=True)
# categories = the label column, WITHOUT the header row
chart.set_categories(Reference(ws, min_col=1, min_row=4, max_row=15))
chart.width, chart.height = 12, 7  # centimeters (see placement rule below)
ws.add_chart(chart, "B8")          # anchor = cell string; add_chart builds the anchor
```

- **Multiple series**: widen the data `Reference` across several value
  columns (`min_col`..`max_col`); each column becomes one series.
- **LineChart**: same API, no `.type`.
- **PieChart**: one series only; `set_categories` = the slice labels.
- **Axis number format**: `chart.y_axis.numFmt = "₩#,##0"`.

`skill.save()` auto-resolves overlaps afterward (see placement rule below).

### ⚠️ Chart data rows — don't hide them

Excel charts default to `plotVisOnly=1`. Hidden helper rows make the
chart render **empty**.

```text
❌ ws.row_dimensions[r].hidden = True   # for rows referenced by a chart
✅ leave helper rows visible (on a separate sheet, or below the visible content)
```

### ⚠️ Chart placement — avoid overlap

`chart.width`/`chart.height` are in **centimeters**, not column units.
1 default column ≈ 2.3cm. A 15cm chart spans ~7 columns.

**Rule: side-by-side only if `chart.width ≤ 12cm`. Otherwise stack vertically.**

```text
❌ c1.width=c2.width=15; ws.add_chart(c1,"B7"); ws.add_chart(c2,"H7")
   # 15cm × 2 → overlap on column H

✅ vertical stack: ws.add_chart(c1,"B7"); ws.add_chart(c2,"B27")
✅ side-by-side only when each ≤ 12cm
```

Next-row math: `next_row = anchor_row + ceil(height_cm * 2) + 2`.

### ⚠️ Chart vs cell data — don't put data under a chart

A chart's anchor area visually covers cells beneath it. If those cells
hold the data the chart reads, the data is invisible to the user.

```text
❌ ws.add_chart(c, "B7")   # 15cm chart spans B7..H22
   ws["B30"] = "Month"     # data table at B30..B36 sits under chart 2 at B27
✅ Put data tables to the right of charts (e.g. M30..N36) or below.
```

`xlsx_skill.save()` warns on stderr if any populated cell falls inside
a chart's footprint.

### ⚠️ Freeze pane — line must not split a data block

The row directly above the freeze line must be a **blank spacer** (or
the freeze line must sit just below a lone header row). Otherwise the
line cuts through a content block and the data ends up half-frozen,
half-scrolling.

```text
❌ Helper table at P3:Q15 + freeze=A7
   → rows 3..6 frozen, rows 7..15 scroll → same table split

✅ Move freeze above the table (e.g. A3, only title fixed),
   or move the table to start at/after the freeze line,
   or drop freeze on dashboard-style sheets.
```

`xlsx_skill.save()` warns if the row above the freeze line is part of
a multi-row data block.

### ⚠️ PivotTable — don't create with openpyxl

openpyxl can write a PivotTable definition but **cannot fill the result
cells** (Excel computes them on refresh). Output looks empty until the
user clicks refresh.

```text
❌ openpyxl.pivot.table.PivotTable(...)   # cells stay blank
✅ Build a summary table with SUMIFS / SUMIF instead
```

## `Formula` — declarative formula builder

Always prefer these over raw `"=SUM(...)"` strings.

| Method | Returns |
|---|---|
| `Formula.sum(range_ref)` | `"=SUM({range_ref})"` |
| `Formula.average(range_ref)` | `"=AVERAGE({range_ref})"` |
| `Formula.safe_divide(a, b)` | `"=IFERROR({a}/{b}, 0)"` (zero-safe) |
| `Formula.rank(cell, range, descending=True)` | `"=RANK(...)"` (classic; no prefix needed) |
| `Formula.rank_eq(cell, range, descending=True)` | `"=_xlfn.RANK.EQ(...)"` |
| `Formula.rank_avg(cell, range, descending=True)` | `"=_xlfn.RANK.AVG(...)"` |
| `Formula.ifs(*cond_result_pairs)` | Compiles to nested `IF(...)`. Pass alternating `cond, result, ...`; last `"TRUE"` becomes the else-branch. |
| `Formula.xlookup(lookup, lookup_arr, return_arr, if_not_found='""')` | `"=_xlfn.XLOOKUP(...)"` — Excel 2019+ only. Prefer `INDEX(MATCH(...))`. |
| `Formula.maxifs(max_range, *criteria_pairs)` | `"=_xlfn.MAXIFS(...)"` — Excel 2019+ |
| `Formula.minifs(min_range, *criteria_pairs)` | `"=_xlfn.MINIFS(...)"` — Excel 2019+ |
| `Formula.ifna(value, if_na_value)` | `"=_xlfn.IFNA(...)"` — Excel 2013+. Use `IFERROR` for older. |
| `Formula.concat(*range_refs)` | `"=_xlfn.CONCAT(...)"` — Excel 2019+. Use `CONCATENATE` or `&`. |
| `Formula.sumifs_month(sum_range, date_range, year, month)` | Locale-safe monthly `SUMIFS` using `DATE()` + `EOMONTH()`. |
| `Formula.sumifs_date_range(sum_range, date_range, (y,m,d), (y,m,d))` | Locale-safe arbitrary-range `SUMIFS`. |

### ⚠️ SUMIFS/COUNTIFS date criteria

```text
❌ =SUMIFS(amount, date_col, ">=2026-01-01", date_col, "<=2026-01-31")  # non-English locale Excel returns 0
✅ =SUMIFS(amount, date_col, ">="&DATE(2026,1,1), date_col, "<="&EOMONTH(DATE(2026,1,1),0))
```
→ Use `Formula.sumifs_month()` / `sumifs_date_range()`.

**Also: date cells must be real date values, not text.** `SUMIFS` with
`DATE()` silently returns 0 against a text date column. `add_sheet` rows
auto-convert ISO strings, but if you write cells directly use a
`datetime.date`/`datetime` (not `d.strftime(...)`). `read_computed()` will
surface a wrong 0 here.

### ⚠️ Excel version compatibility

- **Excel 2010+ functions** (`RANK.EQ`, `XLOOKUP`, `MAXIFS`, `IFNA`,
  `CONCAT`, ...) need the `_xlfn.` prefix or Excel shows `#NAME?`. The
  `Formula` helpers add it.
- **Excel 2019/365 functions** (`IFS`, `XLOOKUP`, `TEXTJOIN`, `FILTER`,
  `SORT`, `UNIQUE`, ...) show `#NAME?` in Excel 2016 and earlier even with
  the prefix — prefer the compatible form noted in the `Formula` table
  above (`IFS` → nested `IF`, `XLOOKUP` → `INDEX(MATCH)`, `IFNA` →
  `IFERROR`, `CONCAT` → `&`).

`verify_formulas()` catches any `#NAME?` that slips through.
