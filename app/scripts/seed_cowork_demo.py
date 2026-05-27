#!/usr/bin/env python3
"""Seed a SQLite database for the Cowork demo.

The script delegates user creation to the backend binary so password hashing
stays identical to the production Argon2 implementation. Projects, sessions,
messages, and uploaded files are written directly so the demo opens with a
realistic state.
"""
from __future__ import annotations

import argparse
import os
import shutil
import sqlite3
import subprocess
from pathlib import Path
from datetime import datetime, timezone, timedelta
import json

ROOT = Path(__file__).resolve().parents[2]
APP_DIR = ROOT / "app"
DEFAULT_DB = APP_DIR / ".demo" / "cowork-demo.db"
DEFAULT_DATA_ROOT = APP_DIR / ".demo" / "files"
DEMO_USERNAME = "olive"
DEMO_PASSWORD = "cowork-demo"
OLIVE_ID = "11111111-1111-4111-8111-111111111111"
MILO_ID = "22222222-2222-4222-8222-222222222222"
OWEN_ID = "33333333-3333-4333-8333-333333333333"
DEMO_USERS = [
    {"id": OLIVE_ID, "username": "olive", "display_name": "Olive Park", "role": "admin"},
    {"id": MILO_ID, "username": "milo", "display_name": "Milo Chen", "role": "user"},
    {"id": OWEN_ID, "username": "owen", "display_name": "Owen Mathers", "role": "user"},
]
PROJECT_KLIENT = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
PROJECT_GTM = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
SESSION_Q2 = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
SESSION_DECISION = "aaaaaaaa-2222-4222-8222-aaaaaaaaaaaa"
SESSION_ATTACHED = "aaaaaaaa-3333-4333-8333-aaaaaaaaaaaa"  # message with attachment
SESSION_GTM = "bbbbbbbb-1111-4111-8111-bbbbbbbbbbbb"
SESSION_REPORT = "bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb"   # session with artifacts


def now(offset: int = 0) -> str:
    return (datetime.now(timezone.utc) + timedelta(seconds=offset)).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def message_json(role: str, text: str) -> str:
    return json.dumps({"role": role, "contents": [{"type": "text", "text": text}]}, ensure_ascii=False)


def sqlite_url(path: Path) -> str:
    return f"sqlite://{path}"


def run_create_admin(db: Path) -> None:
    env = os.environ.copy()
    env["DATABASE_URL"] = sqlite_url(db)
    env.setdefault("AGENT_K_JWT_SECRET", "cowork-demo-secret-change-me")
    result = subprocess.run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "agent-k-backend",
            "--",
            "create-admin",
            "--username",
            DEMO_USERNAME,
            "--password",
            DEMO_PASSWORD,
            "--display-name",
            "Olive Park",
        ],
        cwd=ROOT,
        env=env,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        raise SystemExit(result.stderr or result.stdout)



def user_exists(db: Path, username: str) -> bool:
    if not db.exists():
        return False
    try:
        conn = sqlite3.connect(db)
        row = conn.execute(
            "SELECT 1 FROM users WHERE username = ? LIMIT 1",
            (username,),
        ).fetchone()
        conn.close()
        return row is not None
    except sqlite3.Error:
        return False


def reset_paths(db: Path, data_root: Path) -> None:
    for suffix in ("", "-wal", "-shm"):
        candidate = Path(f"{db}{suffix}")
        if candidate.exists():
            candidate.unlink()
    if data_root.exists():
        shutil.rmtree(data_root)
    db.parent.mkdir(parents=True, exist_ok=True)
    data_root.mkdir(parents=True, exist_ok=True)


def write_files(data_root: Path) -> None:
    # ── shared files (previously "uploads") ──────────────────────────────────
    shared_files = {
        PROJECT_KLIENT: {
            "Market research/Q2 market report.md": "# Q2 market report\n\nSMB renewal cycle shortened by 18%. Proof-led onboarding language is recommended.\n",
            "Market research/Competitor scan raw.csv": "vendor,tier,signal\nNorthstar,usage-based,enterprise\nAtlas,seat-minimum,enterprise\n",
            "Client materials/Revenue cohort.csv": "segment,change\nSMB renewal,-11.4\nEnterprise upsell,+4.1\n",
            "Drafts/Board memo v3.md": "# Board memo v3\n\nOpen slot: market evidence for SMB retention priority.\n",
        },
        PROJECT_GTM: {
            "Launch/H2 launch brief.md": "# H2 launch brief\n\nLaunch window starts late July. Enterprise proof needs a separate appendix.\n",
            "Launch/ICP message matrix.csv": "icp,message\nMid-market,proof-led onboarding\nEnterprise,governance narrative\n",
        },
    }
    for project_id, entries in shared_files.items():
        root = data_root / "projects" / project_id / "shared"
        for rel, content in entries.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")

    # ── inputs: per-session attached files ───────────────────────────────────
    inputs_files = {
        # SESSION_ATTACHED: user attached a raw survey CSV before asking for analysis
        (PROJECT_KLIENT, SESSION_ATTACHED): {
            "survey_raw.csv": (
                "respondent_id,renewal_intent,pain_point\n"
                "R001,renew,onboarding too long\n"
                "R002,churn,pricing unclear\n"
                "R003,renew,great support\n"
                "R004,churn,missing integrations\n"
                "R005,renew,easy to use\n"
            ),
        },
    }
    for (project_id, session_id), entries in inputs_files.items():
        root = data_root / "projects" / project_id / "sessions" / session_id / "inputs"
        for rel, content in entries.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")

    # ── artifacts: agent-generated output files ───────────────────────────────
    artifact_files = {
        # SESSION_REPORT: agent produced a GTM summary report
        (PROJECT_GTM, SESSION_REPORT): {
            "GTM_summary_report.md": (
                "# GTM 재설계 요약 보고서\n\n"
                "## ICP 우선순위\n"
                "1. Mid-market — proof-led onboarding\n"
                "2. Enterprise — governance narrative\n\n"
                "## 런치 타임라인\n"
                "- 7월 말: 소프트 런치 (Mid-market)\n"
                "- 8월 중: 엔터프라이즈 appendix 추가\n\n"
                "## 권장 액션\n"
                "- ICP별 메시지 매트릭스를 sales deck에 반영\n"
                "- 엔터프라이즈 증거 자료 확보 우선\n"
            ),
            "ICP_comparison_table.csv": (
                "icp,priority,message,evidence_needed\n"
                "Mid-market,1,proof-led onboarding,case study x2\n"
                "Enterprise,2,governance narrative,security audit report\n"
                "SMB,3,ease of use,short video demo\n"
            ),
        },
    }
    for (project_id, session_id), entries in artifact_files.items():
        root = data_root / "projects" / project_id / "sessions" / session_id / "artifacts"
        for rel, content in entries.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")


def seed_rows(db: Path) -> None:
    # Fix the base time so all timestamps in this seed run are consistent.
    base = datetime.now(timezone.utc)
    def ts(offset: int = 0) -> str:
        return (base + timedelta(seconds=offset)).isoformat(timespec="milliseconds").replace("+00:00", "Z")

    conn = sqlite3.connect(db)
    conn.execute("PRAGMA foreign_keys = ON")
    olive_hash = conn.execute("SELECT password_hash FROM users WHERE username = ?", (DEMO_USERNAME,)).fetchone()[0]
    created = ts()
    users = [
        (user["id"], user["username"], olive_hash, user["role"], user["display_name"], 1, created, created)
        for user in DEMO_USERS
    ]
    conn.execute("DELETE FROM session_reads")
    conn.execute("DELETE FROM session_messages")
    conn.execute("DELETE FROM sessions")
    conn.execute("DELETE FROM project_members")
    conn.execute("DELETE FROM projects")
    conn.execute("DELETE FROM users")
    conn.executemany(
        "INSERT INTO users (id, username, password_hash, role, display_name, is_active, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        users,
    )
    projects = [
        (PROJECT_KLIENT, "KlientCo Q2 분석", "시장 분석 + Q2 보드 보고 자료 정리", OLIVE_ID, ts(1), ts(1)),
        (PROJECT_GTM, "GTM 재설계 — 2026 H2", "메시지, ICP, launch sequence를 다시 묶는 team project", MILO_ID, ts(2), ts(2)),
    ]
    conn.executemany("INSERT INTO projects (id, name, description, owner_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)", projects)
    members = [
        (PROJECT_KLIENT, MILO_ID, ts(3)),
        (PROJECT_KLIENT, OWEN_ID, ts(4)),
        (PROJECT_GTM, OLIVE_ID, ts(5)),
    ]
    conn.executemany("INSERT INTO project_members (project_id, user_id, added_at) VALUES (?, ?, ?)", members)

    # last_message_at and last_message_snippet are derived from the last message per session.
    # Offsets match the message timestamps below so the values are consistent.
    sessions = [
        (
            SESSION_Q2, PROJECT_KLIENT, OLIVE_ID, "shared_chat",
            "Q2 시장 분석 시작점",
            ts(31),
            "수요 측 갱신 압박부터 보고, 경쟁사 스캔과 교차 검증하면 좋을 것 같아요. SMB 갱신 사이클이 18% 단축됐다는 신호가 가장 강합니다.",
            ts(10), ts(31),
        ),
        (
            SESSION_DECISION, PROJECT_KLIENT, OLIVE_ID, "shared_chat",
            "보드 메모 결정 누적",
            ts(33),
            "현재 결정 스레드는 SMB retention을 최우선으로 두는 방향입니다. 메모 v3에 'market evidence for SMB retention priority' 슬롯을 채울 준비가 됐어요.",
            ts(11), ts(33),
        ),
        (
            SESSION_ATTACHED, PROJECT_KLIENT, OLIVE_ID, "shared_chat",
            "설문 데이터 분석 요청",
            ts(36),
            "survey_raw.csv 기반으로 이탈 위험 응답자의 공통 pain point를 추출하면 'pricing unclear'와 'missing integrations'가 주요 원인입니다.",
            ts(12), ts(36),
        ),
        (
            SESSION_GTM, PROJECT_GTM, MILO_ID, "shared_chat",
            "H2 ICP 메시지 순서 검토",
            ts(37),
            "H2 launch sequence에서 ICP별 메시지 순서를 다시 보고 싶어.",
            ts(13), ts(37),
        ),
        (
            SESSION_REPORT, PROJECT_GTM, MILO_ID, "shared_chat",
            "GTM 요약 보고서 생성",
            ts(40),
            "GTM_summary_report.md와 ICP_comparison_table.csv를 Artifacts에 저장했습니다. 보고서에는 ICP 우선순위와 런치 타임라인이 포함되어 있습니다.",
            ts(14), ts(40),
        ),
    ]
    conn.executemany(
        "INSERT INTO sessions (id, project_id, creator_id, share_mode, title, last_message_at, last_message_snippet, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        sessions,
    )

    # Global paths for attached files (stored in session_messages.attachments as JSON array)
    attached_input_path = f"projects/{PROJECT_KLIENT}/sessions/{SESSION_ATTACHED}/inputs/survey_raw.csv"

    # (session_id, message_json, created_at, sender_kind, sender_name, sender_user_id, attachments_json)
    def user_msg(session_id: str, text: str, creator_id: str, t: str, attachments: list[str] | None = None):
        return (session_id, message_json("user", text), t, "user", None, creator_id, json.dumps(attachments or []), "[]")

    def agent_msg(session_id: str, text: str, t: str, artifacts: list[str] | None = None):
        return (session_id, message_json("assistant", text), t, "agent", "agent-k", None, "[]", json.dumps(artifacts or []))

    messages = [
        # SESSION_Q2
        user_msg(SESSION_Q2, "Q2 시장 보고를 어디서 시작하면 좋을까? Files → Market research에 자료가 정리되어 있어.", OLIVE_ID, ts(30)),
        agent_msg(SESSION_Q2, "수요 측 갱신 압박부터 보고, 경쟁사 스캔과 교차 검증하면 좋을 것 같아요. SMB 갱신 사이클이 18% 단축됐다는 신호가 가장 강합니다.", ts(31)),
        # SESSION_DECISION
        user_msg(SESSION_DECISION, "오늘 결정된 내용을 board memo에 붙일 수 있게 누적해줘.", OLIVE_ID, ts(32)),
        agent_msg(SESSION_DECISION, "현재 결정 스레드는 SMB retention을 최우선으로 두는 방향입니다. 메모 v3에 'market evidence for SMB retention priority' 슬롯을 채울 준비가 됐어요.", ts(33)),
        # SESSION_ATTACHED — user attached a CSV, agent referenced it
        user_msg(
            SESSION_ATTACHED,
            "첨부한 설문 데이터에서 이탈 위험 응답자들의 공통 pain point를 찾아줘.",
            OLIVE_ID,
            ts(35),
            attachments=[attached_input_path],
        ),
        agent_msg(SESSION_ATTACHED, "survey_raw.csv 기반으로 이탈 위험 응답자의 공통 pain point를 추출하면 'pricing unclear'와 'missing integrations'가 주요 원인입니다.", ts(36)),
        # SESSION_GTM
        user_msg(SESSION_GTM, "H2 launch sequence에서 ICP별 메시지 순서를 다시 보고 싶어.", MILO_ID, ts(37)),
        # SESSION_REPORT — agent produced artifact files
        user_msg(SESSION_REPORT, "ICP 우선순위와 런치 타임라인을 정리한 보고서를 artifacts에 저장해줘.", MILO_ID, ts(38)),
        agent_msg(
            SESSION_REPORT,
            "GTM_summary_report.md와 ICP_comparison_table.csv를 Artifacts에 저장했습니다. 보고서에는 ICP 우선순위와 런치 타임라인이 포함되어 있습니다.",
            ts(40),
            artifacts=["GTM_summary_report.md", "ICP_comparison_table.csv"],
        ),
    ]
    conn.executemany(
        "INSERT INTO session_messages (session_id, message_json, created_at, sender_kind, sender_name, sender_user_id, attachments, artifacts) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        messages,
    )
    conn.commit()
    conn.close()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", type=Path, default=DEFAULT_DB)
    parser.add_argument("--data-root", type=Path, default=DEFAULT_DATA_ROOT)
    parser.add_argument("--no-reset", action="store_true", help="Keep existing DB/files before seeding")
    args = parser.parse_args()
    db = args.db.resolve()
    data_root = args.data_root.resolve()
    if not args.no_reset:
        reset_paths(db, data_root)
    if user_exists(db, DEMO_USERNAME):
        print(f"Reusing existing demo login user: {DEMO_USERNAME}")
    else:
        run_create_admin(db)
    seed_rows(db)
    write_files(data_root)
    print("Cowork demo seed ready")
    print(f"DATABASE_URL=sqlite://{db}")
    print(f"AGENT_K_DATA_ROOT={data_root}")
    print("AGENT_K_JWT_SECRET=cowork-demo-secret-change-me")
    print("BIND_ADDR=127.0.0.1:8080")
    print("Demo users:")
    for user in DEMO_USERS:
        print(
            f"  - username={user['username']} password={DEMO_PASSWORD} "
            f"display_name=\"{user['display_name']}\" role={user['role']} id={user['id']}"
        )
    print("Run backend:")
    print(f"  DATABASE_URL=sqlite://{db} AGENT_K_DATA_ROOT={data_root} AGENT_K_JWT_SECRET=cowork-demo-secret-change-me BIND_ADDR=127.0.0.1:8080 cargo run -p agent-k-backend -- serve")
    print("Run backend + app together:")
    print("  app/scripts/run_cowork_demo.sh")
    print("Run app only:")
    print("  VITE_BACKEND_V2_URL=http://127.0.0.1:8080 pnpm -C app dev")


if __name__ == "__main__":
    main()
