

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

1. On `feat/speedwagon-knowledge`(or merged main), upload one folder's `en/` (or `ko/`) files
   into a project's knowledge folder.
2. Send the matching Speedwagon prompt from the table above.
3. It passes if the reply states the **expected answer** and cites the **answer
   file**, without being misled by the decoys (e.g. answering with the offsite
   date, a competitor's revenue, or another source's top complaint = fail).
