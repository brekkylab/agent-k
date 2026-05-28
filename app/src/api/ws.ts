import { getBaseUrl, getToken, notifyUnauthorized } from './client';

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

// WS auth close codes: 4401 is app-defined; 1008 is Policy Violation (RFC 6455).
const AUTH_CLOSE_CODES = new Set([4401, 1008]);
// Detect auth failure by consecutive rapid closes (backend may not send a specific code).
const RAPID_CLOSE_THRESHOLD_MS = 1000;
const RAPID_CLOSE_MAX = 3;

class AppWebSocketManager {
  private ws: WebSocket | null = null;
  private handlers = new Set<Handler>();
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private active = false;
  private rapidCloseCount = 0;
  private lastCloseTime = 0;

  connect(token: string): void {
    this.active = true;
    this.rapidCloseCount = 0;
    this.lastCloseTime = 0;
    if (this.ws?.readyState === WebSocket.OPEN || this.ws?.readyState === WebSocket.CONNECTING) return;
    const url = `${toWsUrl(getBaseUrl())}/ws?token=${encodeURIComponent(token)}`;
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

      // Explicit auth close codes — stop reconnecting and trigger logout.
      if (AUTH_CLOSE_CODES.has(evt.code)) {
        this.active = false;
        notifyUnauthorized(401, { error: 'invalid token' });
        return;
      }

      // Rapid-close heuristic: if the connection keeps dropping within 1s of
      // opening, assume the token is invalid and bail instead of looping forever.
      const now = Date.now();
      if (now - this.lastCloseTime < RAPID_CLOSE_THRESHOLD_MS) {
        this.rapidCloseCount += 1;
      } else {
        this.rapidCloseCount = 1;
      }
      this.lastCloseTime = now;

      if (this.rapidCloseCount >= RAPID_CLOSE_MAX && getToken()) {
        this.active = false;
        notifyUnauthorized(401, { error: 'invalid token' });
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

export const appWs = new AppWebSocketManager();
