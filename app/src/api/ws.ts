import { ApiError, getBaseUrl, getToken, notifyUnauthorized } from './client';
import { getMe } from './auth';  // Task 2에서 사용, 여기서 미리 추가

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

// 앱이 정의한 WS 인증 실패 코드 (4401만 — 1008은 프록시가 발생시키는 범용 코드라 제외)
const AUTH_CLOSE_CODES = new Set([4401]);
// 접속이 이 시간(ms) 이내에 끊기면 short-lived로 간주
const RAPID_CLOSE_THRESHOLD_MS = 2000;
const RAPID_CLOSE_MAX = 3;


class AppWebSocketManager {
  private ws: WebSocket | null = null;
  private handlers = new Set<Handler>();
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private active = false;
  private rapidCloseCount = 0;
  private connectionStartTime = 0;  // lastCloseTime 대체: WS 생성 시점 기록

  connect(token: string): void {
    this.active = true;
    // early-return 이후에 connectionStartTime 설정 — OPEN/CONNECTING이면 새 소켓 없음
    if (this.ws?.readyState === WebSocket.OPEN || this.ws?.readyState === WebSocket.CONNECTING) return;
    const url = `${toWsUrl(getBaseUrl())}/ws?token=${encodeURIComponent(token)}`;
    this.connectionStartTime = Date.now();  // 실제 소켓 생성 직전에 기록
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

      // onclose가 발생한 시점에 this.ws를 null로 — 재연결 시 early-return 방지
      this.ws = null;

      // 앱 정의 인증 실패 코드 — 재연결 중단 후 HTTP fallback으로 에러 분류
      if (AUTH_CLOSE_CODES.has(evt.code)) {
        this.active = false;
        getMe().then(
          () => {
            // WS는 닫혔지만 HTTP 인증은 통과 → WS 네트워크 문제, 로그아웃 안 함
          },
          (err: unknown) => {
            if (err instanceof ApiError && err.status === 401) {
              notifyUnauthorized(401, { error: err.message });
            }
            // 5xx, 네트워크 오류: 로그아웃 안 함
          }
        );
        return;
      }

      // 접속 유지 시간으로 short-lived 여부 판단
      const lifetime = Date.now() - this.connectionStartTime;
      if (lifetime < RAPID_CLOSE_THRESHOLD_MS) {
        this.rapidCloseCount += 1;
      } else {
        this.rapidCloseCount = 0;  // 충분히 오래 살았으면 카운터 리셋
      }

      if (this.rapidCloseCount >= RAPID_CLOSE_MAX && getToken()) {
        this.active = false;
        getMe().then(
          () => { /* HTTP 통과 → WS만 불안정, 로그아웃 안 함 */ },
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
    this.rapidCloseCount = 0;  // 명시적 disconnect 시 카운터 리셋
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
