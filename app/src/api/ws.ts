import { BASE_URL, getToken } from './client';

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

class AppWebSocketManager {
  private ws: WebSocket | null = null;
  private handlers = new Set<Handler>();
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private active = false;

  connect(token: string): void {
    this.active = true;
    if (this.ws?.readyState === WebSocket.OPEN || this.ws?.readyState === WebSocket.CONNECTING) return;
    const url = `${toWsUrl(BASE_URL)}/ws?token=${encodeURIComponent(token)}`;
    const ws = new WebSocket(url);
    this.ws = ws;

    ws.onmessage = (evt) => {
      try {
        const data = JSON.parse(evt.data as string) as AppWsEvent;
        this.handlers.forEach((h) => h(data));
      } catch { /* noop */ }
    };

    ws.onclose = () => {
      if (!this.active) return;
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
