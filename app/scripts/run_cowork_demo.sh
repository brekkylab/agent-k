#!/usr/bin/env bash
# Seed the Cowork demo DB, start backend-v2, then start the app pointed at it.
# Opens the printed app URL and tells you the login values.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEFAULT_DB="$ROOT/app/.demo/cowork-demo.db"
DEFAULT_DATA_ROOT="$ROOT/app/.demo/files"

DB="$DEFAULT_DB"
DATA_ROOT="$DEFAULT_DATA_ROOT"
BACKEND_PORT="8080"
APP_PORT="4110"
NO_RESET="0"
OPEN_BROWSER="0"

usage() {
  cat <<'EOF'
Usage: app/scripts/run_cowork_demo.sh [options]

Seeds the Cowork demo DB, starts backend-v2, then starts the app pointed at
that backend. Open the printed app URL to use the live frontend.

Options:
  --no-reset             Keep the existing demo DB/files before seeding.
  --backend-port PORT    Backend port. Default: 8080.
  --app-port PORT        Vite app port. Default: 4110.
  --db PATH              SQLite DB path. Default: app/.demo/cowork-demo.db.
  --data-root PATH       Upload/data root. Default: app/.demo/files.
  --open                 Open the app in the default browser after it is ready.
  -h, --help             Show this help.

Examples:
  app/scripts/run_cowork_demo.sh
  app/scripts/run_cowork_demo.sh --no-reset --backend-port 18080 --app-port 14110
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-reset) NO_RESET="1"; shift ;;
    --backend-port) BACKEND_PORT="${2:?--backend-port requires a value}"; shift 2 ;;
    --app-port)     APP_PORT="${2:?--app-port requires a value}";         shift 2 ;;
    --db)           DB="${2:?--db requires a value}";                     shift 2 ;;
    --data-root)    DATA_ROOT="${2:?--data-root requires a value}";       shift 2 ;;
    --open)         OPEN_BROWSER="1"; shift ;;
    -h|--help)      usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

DB="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).expanduser().resolve())' "$DB")"
DATA_ROOT="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).expanduser().resolve())' "$DATA_ROOT")"
BACKEND_URL="http://127.0.0.1:$BACKEND_PORT"
APP_URL="http://127.0.0.1:$APP_PORT"
JWT_SECRET="cowork-demo-secret-change-me"

BACKEND_PID=""
APP_PID=""

cleanup() {
  local exit_code=$?
  trap - EXIT INT TERM
  echo
  echo "Stopping Cowork demo..."
  if [[ -n "$APP_PID" ]] && kill -0 "$APP_PID" 2>/dev/null; then
    kill "$APP_PID" 2>/dev/null || true
  fi
  if [[ -n "$BACKEND_PID" ]] && kill -0 "$BACKEND_PID" 2>/dev/null; then
    kill "$BACKEND_PID" 2>/dev/null || true
  fi
  wait "$APP_PID" 2>/dev/null || true
  wait "$BACKEND_PID" 2>/dev/null || true
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

wait_for_url() {
  local label="$1"
  local url="$2"
  local pid="$3"
  local attempts="${4:-60}"
  local i
  for ((i = 1; i <= attempts; i++)); do
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "$label process exited before becoming ready." >&2
      return 1
    fi
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "$label did not become ready at $url within ${attempts}s." >&2
  return 1
}

echo "==> Seeding Cowork demo DB"
SEED_ARGS=(--db "$DB" --data-root "$DATA_ROOT")
if [[ "$NO_RESET" == "1" ]]; then
  SEED_ARGS+=(--no-reset)
fi
python3 "$ROOT/app/scripts/seed_cowork_demo.py" "${SEED_ARGS[@]}"

echo
echo "==> Starting backend-v2 on $BACKEND_URL"
(
  cd "$ROOT"
  DATABASE_URL="sqlite://$DB" \
  AGENT_K_DATA_ROOT="$DATA_ROOT" \
  AGENT_K_JWT_SECRET="$JWT_SECRET" \
  BIND_ADDR="127.0.0.1:$BACKEND_PORT" \
  cargo run -p agent-k-backend -- serve
) &
BACKEND_PID=$!

wait_for_url "backend-v2" "$BACKEND_URL/docs" "$BACKEND_PID" 90

echo
echo "==> Starting app on $APP_URL"
(
  cd "$ROOT"
  VITE_BACKEND_V2_URL="$BACKEND_URL" pnpm -C app exec vite --host 127.0.0.1 --port "$APP_PORT"
) &
APP_PID=$!

wait_for_url "app" "$APP_URL" "$APP_PID" 45

cat <<EOF

Cowork demo is ready.

App:      $APP_URL
Backend:  $BACKEND_URL
DB:       $DB
Files:    $DATA_ROOT

Login form values:
  Backend URL: $BACKEND_URL
  olive / cowork-demo  (Olive Park, admin)
  milo  / cowork-demo  (Milo Chen, user)
  owen  / cowork-demo  (Owen Mathers, user)

Press Ctrl-C to stop both backend-v2 and the app.
EOF

if [[ "$OPEN_BROWSER" == "1" ]]; then
  if command -v open >/dev/null 2>&1; then
    open "$APP_URL"
  else
    echo "--open requested, but the macOS 'open' command was not found." >&2
  fi
fi

while true; do
  if ! kill -0 "$BACKEND_PID" 2>/dev/null; then
    echo "backend-v2 exited." >&2
    exit 1
  fi
  if ! kill -0 "$APP_PID" 2>/dev/null; then
    echo "app exited." >&2
    exit 1
  fi
  sleep 1
done
