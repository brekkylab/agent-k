#!/usr/bin/env python3
"""Seed automation mock data into the Cowork demo database.

Inserts a handful of automations, triggers, runs (with backing sessions),
and run events scoped to the KlientCo Q2 project from seed_cowork_demo.py.

Run AFTER seed_cowork_demo.py — depends on users / projects existing.

Re-running is idempotent: it wipes the automation subtree (and sessions
tagged origin='automation') before inserting the fresh fixture set.
"""
from __future__ import annotations

import argparse
import json
import sqlite3
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
APP_DIR = ROOT / "app"
DEFAULT_DB = APP_DIR / ".demo" / "cowork-demo.db"

# Reuse identifiers from seed_cowork_demo.py
OLIVE_ID = "11111111-1111-4111-8111-111111111111"
PROJECT_KLIENT = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"

AUTO_A1 = "cccc1111-aaaa-4aaa-8aaa-cccccccccccc"
AUTO_A2 = "cccc2222-aaaa-4aaa-8aaa-cccccccccccc"
AUTO_A3 = "cccc3333-aaaa-4aaa-8aaa-cccccccccccc"
AUTO_A4 = "cccc4444-aaaa-4aaa-8aaa-cccccccccccc"

TRIG_A1 = "dddd1111-aaaa-4aaa-8aaa-dddddddddddd"
TRIG_A2 = "dddd2222-aaaa-4aaa-8aaa-dddddddddddd"
TRIG_A3 = "dddd3333-aaaa-4aaa-8aaa-dddddddddddd"

# webhook token hash (sha256 hex — dummy 64-char hex satisfies the UNIQUE col)
WEBHOOK_TOKEN_HASH_A3 = "a3" * 32

# Seeded runs use only TERMINAL statuses (succeeded / failed / cancelled).
# A live `queued` or `running` row would be picked up by the worker and
# attempt real execution, so we never seed those.
RUN_A1_1 = "eeee1111-aaaa-4aaa-8aaa-eeeeeeeeeeee"   # cancelled
RUN_A1_2 = "eeee1112-aaaa-4aaa-8aaa-eeeeeeeeeeee"   # succeeded
RUN_A1_3 = "eeee1113-aaaa-4aaa-8aaa-eeeeeeeeeeee"   # succeeded
RUN_A2_1 = "eeee2221-aaaa-4aaa-8aaa-eeeeeeeeeeee"   # succeeded
RUN_A3_1 = "eeee3331-aaaa-4aaa-8aaa-eeeeeeeeeeee"   # failed
RUN_A4_1 = "eeee4441-aaaa-4aaa-8aaa-eeeeeeeeeeee"   # succeeded (manual)

SESS_A1_1 = "ffff1111-aaaa-4aaa-8aaa-ffffffffffff"
SESS_A1_2 = "ffff1112-aaaa-4aaa-8aaa-ffffffffffff"
SESS_A1_3 = "ffff1113-aaaa-4aaa-8aaa-ffffffffffff"
SESS_A2_1 = "ffff2221-aaaa-4aaa-8aaa-ffffffffffff"
SESS_A3_1 = "ffff3331-aaaa-4aaa-8aaa-ffffffffffff"
SESS_A4_1 = "ffff4441-aaaa-4aaa-8aaa-ffffffffffff"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", type=Path, default=DEFAULT_DB)
    args = parser.parse_args()
    db = args.db.resolve()
    if not db.exists():
        raise SystemExit(f"DB not found: {db}. Run seed_cowork_demo.py first.")

    base = datetime.now(timezone.utc)

    def ts(offset_sec: int = 0) -> str:
        return (
            (base + timedelta(seconds=offset_sec))
            .isoformat(timespec="milliseconds")
            .replace("+00:00", "Z")
        )

    def title_ts(offset_sec: int) -> str:
        # Match the backend's deterministic format in
        # `backend/src/repository/automation.rs::automation_session_title`:
        # "{automation_name} · {kind_label} · %Y-%m-%d %H:%M".
        return (base + timedelta(seconds=offset_sec)).strftime("%Y-%m-%d %H:%M")

    conn = sqlite3.connect(db)
    conn.execute("PRAGMA foreign_keys = ON")

    # Wipe automation subtree (idempotent re-runs).
    conn.execute("DELETE FROM automation_run_events")
    conn.execute("DELETE FROM automation_runs")
    conn.execute("DELETE FROM automation_triggers")
    conn.execute("DELETE FROM automations")
    # Drop the sessions we own (origin='automation') so we don't accumulate stale rows.
    conn.execute(
        "DELETE FROM session_messages WHERE session_id IN ("
        "  SELECT id FROM sessions WHERE origin = 'automation'"
        ")"
    )
    conn.execute(
        "DELETE FROM session_reads WHERE session_id IN ("
        "  SELECT id FROM sessions WHERE origin = 'automation'"
        ")"
    )
    conn.execute("DELETE FROM sessions WHERE origin = 'automation'")

    # ── Automations ───────────────────────────────────────────────────────
    automations = [
        (
            AUTO_A1, PROJECT_KLIENT, "Daily summary",
            "어제 진행된 세션 요약과 오늘 우선순위 추출",
            json.dumps([
                "어제 진행된 세션을 1-2줄 요약으로 정리해줘.",
                "오늘 우선순위 항목 3개를 뽑아줘.",
            ], ensure_ascii=False),
            1, OLIVE_ID, ts(1), ts(1),
        ),
        (
            AUTO_A2, PROJECT_KLIENT, "Weekly digest",
            "주간 핵심 결정 정리 (금요일 발송)",
            json.dumps(["이번 주 핵심 결정 사항을 정리해줘."], ensure_ascii=False),
            1, OLIVE_ID, ts(2), ts(2),
        ),
        (
            AUTO_A3, PROJECT_KLIENT, "On-call ping",
            "외부 인시던트 시스템에서 호출하는 핸드오프",
            json.dumps(["인시던트 내용을 받아 1줄 요약과 담당자를 추정해줘."], ensure_ascii=False),
            0, OLIVE_ID, ts(3), ts(3),   # disabled
        ),
        (
            AUTO_A4, PROJECT_KLIENT, "Backfill ledger",
            "수동 실행만 — 누락된 ledger 항목 재처리",
            json.dumps(["주어진 기간의 누락 ledger 항목을 다시 처리해줘."], ensure_ascii=False),
            1, OLIVE_ID, ts(4), ts(4),
        ),
    ]
    conn.executemany(
        "INSERT INTO automations "
        "(id, project_id, name, description, prompts_json, enabled, created_by, created_at, updated_at) "
        "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        automations,
    )

    # ── Triggers ──────────────────────────────────────────────────────────
    triggers = [
        (
            TRIG_A1, AUTO_A1, "cron",
            json.dumps({"expr": "0 10 * * *", "tz": "Asia/Seoul"}),
            1, ts(60 * 60 * 6), None, ts(5), ts(5),
        ),
        (
            TRIG_A2, AUTO_A2, "cron",
            json.dumps({"expr": "0 9 * * 5", "tz": "Asia/Seoul"}),
            1, ts(60 * 60 * 24 * 3), None, ts(6), ts(6),
        ),
        (
            TRIG_A3, AUTO_A3, "webhook",
            json.dumps({}),
            0, None, WEBHOOK_TOKEN_HASH_A3, ts(7), ts(7),
        ),
        # a4 has no triggers (manual-only)
    ]
    conn.executemany(
        "INSERT INTO automation_triggers "
        "(id, automation_id, kind, spec_json, enabled, next_fire_at, webhook_token_hash, created_at, updated_at) "
        "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        triggers,
    )

    # ── Sessions (one per run; origin='automation') ───────────────────────
    # Session titles mirror the backend's deterministic format for
    # automation-created sessions:
    #   "{automation_name} · {kind_label} · {scheduled_for as %Y-%m-%d %H:%M}"
    # kind_label maps 'cron' → 'recurring'; 'webhook' / 'manual' stay verbatim.
    sessions = [
        (SESS_A1_1, PROJECT_KLIENT, OLIVE_ID, "private",
         f"Daily summary · recurring · {title_ts(100)}",
         None, None, ts(100), ts(110), "automation"),
        (SESS_A1_2, PROJECT_KLIENT, OLIVE_ID, "private",
         f"Daily summary · recurring · {title_ts(90)}",
         None, None, ts(120), ts(120), "automation"),
        (SESS_A1_3, PROJECT_KLIENT, OLIVE_ID, "private",
         f"Daily summary · recurring · {title_ts(80)}",
         None, None, ts(80), ts(95), "automation"),
        (SESS_A2_1, PROJECT_KLIENT, OLIVE_ID, "private",
         f"Weekly digest · recurring · {title_ts(50)}",
         None, None, ts(50), ts(75), "automation"),
        (SESS_A3_1, PROJECT_KLIENT, OLIVE_ID, "private",
         f"On-call ping · webhook · {title_ts(40)}",
         None, None, ts(40), ts(60), "automation"),
        (SESS_A4_1, PROJECT_KLIENT, OLIVE_ID, "private",
         f"Backfill ledger · manual · {title_ts(30)}",
         None, None, ts(30), ts(48), "automation"),
    ]
    conn.executemany(
        "INSERT INTO sessions "
        "(id, project_id, creator_id, share_mode, title, "
        " last_message_at, last_message_snippet, created_at, updated_at, origin) "
        "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        sessions,
    )

    # ── Runs ──────────────────────────────────────────────────────────────
    runs = [
        # (id, automation_id, trigger_id, session_id, status, scheduled_for, lease_until, previous_run_id, idempotency_key, created_at, updated_at)
        (RUN_A1_1, AUTO_A1, TRIG_A1, SESS_A1_1, "cancelled",
         ts(100), None, None, None, ts(100), ts(110)),
        (RUN_A1_2, AUTO_A1, TRIG_A1, SESS_A1_2, "succeeded",
         ts(90), None, None, None, ts(90), ts(118)),
        (RUN_A1_3, AUTO_A1, TRIG_A1, SESS_A1_3, "succeeded",
         ts(80), None, None, None, ts(80), ts(95)),
        (RUN_A2_1, AUTO_A2, TRIG_A2, SESS_A2_1, "succeeded",
         ts(50), None, None, None, ts(50), ts(75)),
        (RUN_A3_1, AUTO_A3, TRIG_A3, SESS_A3_1, "failed",
         ts(40), None, None, None, ts(40), ts(60)),
        (RUN_A4_1, AUTO_A4, None, SESS_A4_1, "succeeded",
         ts(30), None, None, None, ts(30), ts(48)),
    ]
    conn.executemany(
        "INSERT INTO automation_runs "
        "(id, automation_id, trigger_id, session_id, status, scheduled_for, lease_until, "
        " previous_run_id, idempotency_key, created_at, updated_at) "
        "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        runs,
    )

    # ── Run events ────────────────────────────────────────────────────────
    def evt(run_id: str, offset: int, kind: str, payload=None):
        return (
            run_id,
            ts(offset),
            kind,
            json.dumps(payload, ensure_ascii=False) if payload is not None else None,
        )

    events = [
        # r1 — cancelled by user mid-flight
        evt(RUN_A1_1, 100, "triggered", {"trigger_id": TRIG_A1, "expr": "0 10 * * *"}),
        evt(RUN_A1_1, 100, "queued"),
        evt(RUN_A1_1, 102, "started", {"worker": "worker-0"}),
        evt(RUN_A1_1, 102, "step_started", {"index": 0}),
        evt(RUN_A1_1, 110, "cancelled", {"reason": "user_requested", "actor_user_id": OLIVE_ID}),
        # r1_2 — succeeded
        evt(RUN_A1_2, 90, "triggered"),
        evt(RUN_A1_2, 90, "queued"),
        evt(RUN_A1_2, 91, "started"),
        evt(RUN_A1_2, 118, "succeeded", {"tokens": 1450}),
        # r1_3 — succeeded
        evt(RUN_A1_3, 80, "triggered"),
        evt(RUN_A1_3, 80, "queued"),
        evt(RUN_A1_3, 81, "started"),
        evt(RUN_A1_3, 95, "succeeded", {"tokens": 1182}),
        # a2 — succeeded
        evt(RUN_A2_1, 50, "triggered"),
        evt(RUN_A2_1, 50, "queued"),
        evt(RUN_A2_1, 51, "started"),
        evt(RUN_A2_1, 75, "succeeded", {"tokens": 812}),
        # a3 — failed
        evt(RUN_A3_1, 40, "triggered", {"bearer": "***"}),
        evt(RUN_A3_1, 40, "queued"),
        evt(RUN_A3_1, 41, "started"),
        evt(RUN_A3_1, 60, "failed", {"reason": "upstream 502"}),
        # a4 — manual succeeded
        evt(RUN_A4_1, 30, "triggered", {"manual": True, "actor": OLIVE_ID}),
        evt(RUN_A4_1, 30, "started"),
        evt(RUN_A4_1, 48, "succeeded"),
    ]
    conn.executemany(
        "INSERT INTO automation_run_events (run_id, ts, kind, payload) VALUES (?, ?, ?, ?)",
        events,
    )

    # ── Session messages ──────────────────────────────────────────────────
    # Drawer preview reads from session_messages via /sessions/:id/messages.
    # `message_json` mirrors seed_cowork_demo.py: `{role, contents:[{type:text,text}]}`.
    def message_json(role: str, text: str) -> str:
        return json.dumps(
            {"role": role, "contents": [{"type": "text", "text": text}]},
            ensure_ascii=False,
        )

    def prompt_msg(session_id: str, text: str, t: str):
        # automation prompts are written as user-role messages, attributed to
        # the automation creator (olive) so they render on the right side.
        return (session_id, message_json("user", text), t, "user", None, OLIVE_ID)

    def agent_msg(session_id: str, text: str, t: str):
        return (session_id, message_json("assistant", text), t, "agent", "agent-k", None)

    session_messages = [
        # a1_1 (cancelled) — prompt fired but agent never completed
        prompt_msg(SESS_A1_1, "어제 진행된 세션을 1-2줄 요약으로 정리해줘.", ts(101)),
        # a1_2 (succeeded) — both prompts ran to completion
        prompt_msg(SESS_A1_2, "어제 진행된 세션을 1-2줄 요약으로 정리해줘.", ts(91)),
        agent_msg(
            SESS_A1_2,
            "어제 활동 요약입니다.\n\n총 **5건의 세션**을 확인했습니다.\n"
            "- Klient kickoff (Olive, 14:02)\n"
            "- GTM brainstorm (Milo, 15:30)\n"
            "- Q2 plan review (Owen, 16:45)\n"
            "- 보드 메모 결정 누적 (Olive, 17:10)\n"
            "- 후속 액션 정리 (Owen, 18:20)\n",
            ts(105),
        ),
        prompt_msg(SESS_A1_2, "오늘 우선순위 항목 3개를 뽑아줘.", ts(106)),
        agent_msg(
            SESS_A1_2,
            "오늘 우선순위 3개\n\n"
            "1. **보드 메모 v4 마감** — 시장 evidence 슬롯 확정\n"
            "2. **GTM ICP 매트릭스 컨펌** — mid-market / enterprise 분기\n"
            "3. **온콜 로테이션 공지** — 페이저 정책 개정안 전파\n",
            ts(118),
        ),
        # a1_3 (succeeded, yesterday) — single-prompt run
        prompt_msg(SESS_A1_3, "어제 진행된 세션을 1-2줄 요약으로 정리해줘.", ts(81)),
        agent_msg(
            SESS_A1_3,
            "어제는 3건의 세션이 있었습니다. 주요 결정: SMB retention 우선순위 확정.",
            ts(95),
        ),
        # a2_1 (succeeded) — weekly digest
        prompt_msg(SESS_A2_1, "이번 주 핵심 결정 사항을 정리해줘.", ts(51)),
        agent_msg(
            SESS_A2_1,
            "이번 주 핵심 결정\n\n"
            "1. **GTM 전환** — 1차 타깃 mid-market 확정\n"
            "2. **Q2 OKR** — 매출 목표 +18%\n"
            "3. **온콜 로테이션** — 페이저 정책 개정안 합의\n\n"
            "증빙: 사내 위키 GTM-2026Q2 페이지.",
            ts(75),
        ),
        # a3_1 (failed) — webhook handoff that errored
        prompt_msg(SESS_A3_1, "인시던트 INC-4471에 대한 상황 요약을 작성해.", ts(41)),
        agent_msg(
            SESS_A3_1,
            "실패: 업스트림에서 **502 Bad Gateway**가 발생했습니다.\n"
            "재시도 1회 — 동일한 502.\n"
            "사람이 한 번 확인이 필요합니다.",
            ts(60),
        ),
        # a4_1 (manual succeeded)
        prompt_msg(SESS_A4_1, "5/15~5/19 누락된 ledger 항목을 다시 처리해.", ts(31)),
        agent_msg(
            SESS_A4_1,
            "**10건**의 누락 항목을 재처리했습니다.\n"
            "- 성공 9건, 충돌 1건 (`txn-2026-05-17-031`)\n"
            "- 충돌 항목은 수동 확인 필요.",
            ts(48),
        ),
    ]
    conn.executemany(
        "INSERT INTO session_messages "
        "(session_id, message_json, created_at, sender_kind, sender_name, sender_user_id) "
        "VALUES (?, ?, ?, ?, ?, ?)",
        session_messages,
    )

    # Sync sessions.last_message_at / last_message_snippet to reflect the
    # final message inserted per session (mirrors what the live message-send
    # path does in the backend).
    conn.execute(
        """
        UPDATE sessions
        SET last_message_at = (
              SELECT created_at FROM session_messages
              WHERE session_id = sessions.id
              ORDER BY created_at DESC LIMIT 1
            ),
            last_message_snippet = (
              SELECT substr(
                json_extract(message_json, '$.contents[0].text'), 1, 160
              )
              FROM session_messages
              WHERE session_id = sessions.id
              ORDER BY created_at DESC LIMIT 1
            )
        WHERE origin = 'automation'
        """
    )

    conn.commit()
    conn.close()

    print(f"Seeded automations into {db}")
    print(f"  - {len(automations)} automations  (a3 disabled)")
    print(f"  - {len(triggers)} triggers")
    print(f"  - {len(runs)} runs ({len(sessions)} backing sessions)")
    print(f"  - {len(events)} events")
    print(f"  - {len(session_messages)} session messages")


if __name__ == "__main__":
    main()
