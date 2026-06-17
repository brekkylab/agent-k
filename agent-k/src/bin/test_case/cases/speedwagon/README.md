# Speedwagon test corpus (manual fixtures)

A hand-built document corpus for exercising the **Speedwagon home suggested
prompts**. These files are **not** wired into the `test_case` harness — they are
reference/demo data, meant to be dropped into a project's knowledge folder and
queried manually with the matching prompt. (Automated corpus-QA cases live with
the harness on `feat/speedwagon-knowledge`.)

Each scenario folder has `en/` and `ko/` (10 files each: **one answer file +
nine decoys**), so the same prompt can be tested in either language.

## Folder → prompt → answer

| Folder | Prompt (en / ko) | Answer file (en / ko) | Expected answer |
|---|---|---|---|
| `Find a date` | "Find the contract or launch dates in the documents." / "계약/런치 일정이 언제인지 문서에서 찾아줘." | `launch_contract_schedule.md` / `계약_런치_일정.md` | contract signed **2026-03-15**, soft launch **2026-07-22**, public launch **2026-08-05** |
| `Find with sources` | "Find last year's revenue in the documents and cite the source." / "작년 매출이 얼마였는지 문서에서 찾아 출처와 함께 알려줘." | `annual_report_fy2025.md` / `FY2025_연차보고서.md` | FY2025 total revenue **$48.2M** |
| `Top complaint` | "Find the most common complaint in the survey responses." / "설문 응답에서 가장 많이 나온 불만이 뭔지 정리해줘." | `customer_survey_results.md` / `고객_설문_결과.md` | **"Onboarding takes too long"** (14 of 36) |

The other nine files in each folder are **decoys** designed so a correct answer
requires picking the right source/figure rather than keyword-matching:

- **Find a date** — a team-offsite note carries a date that is *not* a contract/
  launch date; other files mention "launch" with no date.
- **Find with sources** — other revenue-shaped numbers (this-year Q1, next-year
  target, competitors' revenue, cost lines) appear, but only the annual report
  holds last year's total.
- **Top complaint** — every other feedback source (support tickets, app-store
  reviews, NPS, exit interviews) has a *different* top item; only the survey
  yields the expected answer.

## How to run a scenario

1. On `feat/speedwagon-knowledge` (or merged main), upload one folder's `en/` (or `ko/`) files into a project's knowledge folder.
2. Send the matching Speedwagon prompt from the table above.
3. It passes if the reply states the **expected answer** and cites the **answer
   file**, without being misled by the decoys (e.g. answering with the offsite
   date, a competitor's revenue, or another source's top complaint = fail).
