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
SESSION_GTM = "bbbbbbbb-1111-4111-8111-bbbbbbbbbbbb"


def now(offset: int = 0) -> str:
    return (datetime.now(timezone.utc) + timedelta(seconds=offset)).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def message(role: str, text: str) -> str:
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


def write_uploads(data_root: Path) -> None:
    files = {
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
    for project_id, entries in files.items():
        root = data_root / "projects" / project_id / "uploads"
        for rel, content in entries.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")


def seed_rows(db: Path) -> None:
    conn = sqlite3.connect(db)
    conn.execute("PRAGMA foreign_keys = ON")
    olive_hash = conn.execute("SELECT password_hash FROM users WHERE username = ?", (DEMO_USERNAME,)).fetchone()[0]
    created = now()
    users = [
        (user["id"], user["username"], olive_hash, user["role"], user["display_name"], 1, created, created)
        for user in DEMO_USERS
    ]
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
        (PROJECT_KLIENT, "KlientCo Q2 분석", "시장 분석 + Q2 보드 보고 자료 정리", OLIVE_ID, now(1), now(1)),
        (PROJECT_GTM, "GTM 재설계 — 2026 H2", "메시지, ICP, launch sequence를 다시 묶는 team project", MILO_ID, now(2), now(2)),
    ]
    conn.executemany("INSERT INTO projects (id, name, description, owner_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)", projects)
    members = [
        (PROJECT_KLIENT, MILO_ID, now(3)),
        (PROJECT_KLIENT, OWEN_ID, now(4)),
        (PROJECT_GTM, OLIVE_ID, now(5)),
    ]
    conn.executemany("INSERT INTO project_members (project_id, user_id, added_at) VALUES (?, ?, ?)", members)
    sessions = [
        (SESSION_Q2, PROJECT_KLIENT, OLIVE_ID, "shared_chat", now(10), now(20)),
        (SESSION_DECISION, PROJECT_KLIENT, OLIVE_ID, "shared_chat", now(11), now(21)),
        (SESSION_GTM, PROJECT_GTM, MILO_ID, "shared_chat", now(12), now(22)),
    ]
    conn.executemany("INSERT INTO sessions (id, project_id, creator_id, share_mode, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)", sessions)
    messages = [
        (SESSION_Q2, message("user", "Q2 시장 보고를 어디서 시작하면 좋을까? Files → Market research에 자료가 정리되어 있어."), now(30)),
        (SESSION_Q2, message("assistant", "수요 측 갱신 압박부터 보고, 경쟁사 스캔과 교차 검증하면 좋을 것 같아요. SMB 갱신 사이클이 18% 단축됐다는 신호가 가장 강합니다."), now(31)),
        (SESSION_DECISION, message("user", "오늘 결정된 내용을 board memo에 붙일 수 있게 누적해줘."), now(32)),
        (SESSION_DECISION, message("assistant", "현재 결정 스레드는 SMB retention을 최우선으로 두는 방향입니다. 메모 v3에 'market evidence for SMB retention priority' 슬롯을 채울 준비가 됐어요."), now(33)),
        (SESSION_GTM, message("user", "H2 launch sequence에서 ICP별 메시지 순서를 다시 보고 싶어."), now(34)),
    ]
    conn.executemany("INSERT INTO session_messages (session_id, message_json, created_at) VALUES (?, ?, ?)", messages)
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
    write_uploads(data_root)
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
