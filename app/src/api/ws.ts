import { ApiError, getBaseUrl, getToken, notifyUnauthorized } from './client';
import { getMe } from './auth';

export interface SessionTitleUpdatedEvent {
  type: 'session_title_updated';
  session_id: string;
  project_id: string;
  title: string;
}

export type AppWsEvent = SessionTitleUpdatedEvent;

type Handler = (event: AppWsEvent) => void;

function toWsUrl(httpBase: string): string {
  return httpBase.replace(/^https?/, (m) => (m === 'https' ? 'wss' : 'ws'));
}

// App-defined WS auth failure code. 1008 (Policy Violation) is intentionally excluded
// as proxies and firewalls emit it for non-auth reasons.
const AUTH_CLOSE_CODES = new Set([4401]);
// Connections that close within this window are considered short-lived.
const RAPID_CLOSE_THRESHOLD_MS = 2000;
const RAPID_CLOSE_MAX = 3;


class AppWebSocketManager {
  private ws: WebSocket | null = null;
  private handlers = new Set<Handler>();
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private active = false;
  private rapidCloseCount = 0;
  private connectionStartTime = 0;  // replaces lastCloseTime: records when the socket was created

  connect(token: string): void {
    this.active = true;
    if (this.ws?.readyState === WebSocket.OPEN || this.ws?.readyState === WebSocket.CONNECTING) return;
    const url = `${toWsUrl(getBaseUrl())}/ws?token=${encodeURIComponent(token)}`;
    this.connectionStartTime = Date.now();
    const ws = new WebSocket(url);
    this.ws = ws;

    ws.onmessage = (evt) => {
      try {
        const data = JSON.parse(evt.data as string) as AppWsEvent;
        this.handlers.forEach((h) => h(data));
      } catch { /* noop */ }
    };

    ws.onclose = (evt) => {
      if (!this.active) return;

      // Clear the socket reference so the reconnect call doesn't hit the early-return guard.
      this.ws = null;

      // App-defined auth close code — stop reconnecting and classify the failure via HTTP.
      if (AUTH_CLOSE_CODES.has(evt.code)) {
        this.active = false;
        getMe().then(
          () => {
            // WS closed but HTTP auth passed — treat as a transient WS flap, not an auth failure.
          },
          (err: unknown) => {
            if (err instanceof ApiError && err.status === 401) {
              notifyUnauthorized(401, { error: err.message });
            }
            // 5xx or network error: do not force logout.
          }
        );
        return;
      }

      // Increment counter if the connection was short-lived; reset if it was stable.
      const lifetime = Date.now() - this.connectionStartTime;
      if (lifetime < RAPID_CLOSE_THRESHOLD_MS) {
        this.rapidCloseCount += 1;
      } else {
        this.rapidCloseCount = 0;
      }

      if (this.rapidCloseCount >= RAPID_CLOSE_MAX && getToken()) {
        this.active = false;
        getMe().then(
          () => { /* HTTP auth passed — WS instability only, do not force logout. */ },
          (err: unknown) => {
            if (err instanceof ApiError && err.status === 401) {
              notifyUnauthorized(401, { error: err.message });
            }
          }
        );
        return;
      }

      this.reconnectTimer = setTimeout(() => {
        const t = getToken();
        if (t && this.active) this.connect(t);
      }, 3000);
    };

    ws.onerror = () => { ws.close(); };
  }

  disconnect(): void {
    this.active = false;
    this.rapidCloseCount = 0;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.ws?.close();
    this.ws = null;
  }

  subscribe(handler: Handler): () => void {
    this.handlers.add(handler);
    return () => { this.handlers.delete(handler); };
  }
}

export { AppWebSocketManager };
export const appWs = new AppWebSocketManager();
