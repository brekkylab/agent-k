#!/usr/bin/env bash
# Run the app_v2 stack (backend_v2 + app_v2 frontend) together for local dev.
#
#   ./scripts/dev-v2.sh
#
# - Backend (Rust)  : http://127.0.0.1:8080   (API docs at /docs)
# - Frontend (Vite) : http://localhost:4210
#
# The frontend auto-logs in as `local` / `local-local`. If the SQLite DB has no
# admin yet, the backend creates that account on first boot from the
# AGENT_K_ADMIN_* vars set below. Ctrl+C stops both processes.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Ensure the frontend env exists.
if [[ ! -f app_v2/.env ]]; then
  cp app_v2/.env.example app_v2/.env
  echo "[dev-v2] created app_v2/.env from example"
fi

# Ensure frontend deps are installed.
if [[ ! -d app_v2/node_modules ]]; then
  echo "[dev-v2] installing frontend deps (pnpm install)…"
  (cd app_v2 && pnpm install)
fi

# Kill anything already bound to our ports (a previous run) before starting.
BACKEND_PORT=8080
FRONTEND_PORT=4210
for port in "$BACKEND_PORT" "$FRONTEND_PORT"; do
  stale="$(lsof -ti "tcp:${port}" 2>/dev/null || true)"
  if [[ -n "$stale" ]]; then
    echo "[dev-v2] killing stale process on :${port} (${stale//$'\n'/ })"
    echo "$stale" | xargs kill -9 2>/dev/null || true
  fi
done

# Credentials the frontend auto-login expects; used only if no admin exists yet.
export AGENT_K_ADMIN_USERNAME="${AGENT_K_ADMIN_USERNAME:-local}"
export AGENT_K_ADMIN_PASSWORD="${AGENT_K_ADMIN_PASSWORD:-local-local}"

pids=()
cleanup() {
  echo ""
  echo "[dev-v2] shutting down…"
  for pid in "${pids[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

echo "[dev-v2] starting backend  → http://127.0.0.1:8080 (docs: /docs)"
cargo run -p agent-k-backend &
pids+=($!)

echo "[dev-v2] starting frontend → http://localhost:4210"
(cd app_v2 && pnpm dev) &
pids+=($!)

# Block until both exit; Ctrl+C triggers the cleanup trap. (`wait -n` is avoided
# for compatibility with the bash 3.2 shipped on macOS.)
wait
