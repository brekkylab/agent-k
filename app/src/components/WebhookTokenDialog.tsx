// One-time webhook token reveal dialog. The plaintext bearer token is only
// available immediately after creating a webhook trigger — subsequent reads
// from the backend never include it (only a masked preview). This modal
// reinforces that "save it now" constraint with a copy button and explicit
// acknowledgment to dismiss.

import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '@/components/Icon';

export function WebhookTokenDialog({
  token,
  curlSample,
  onClose,
}: {
  token: string;
  /** Optional ready-made curl example showing the token in use. */
  curlSample?: string;
  onClose: () => void;
}) {
  const [acknowledged, setAcknowledged] = useState(false);
  const [copied, setCopied] = useState(false);
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);

  // Esc closes (only after acknowledge to avoid accidental dismissal).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && acknowledged) onClose();
    };
    window.addEventListener('keydown', onKey);
    closeBtnRef.current?.focus();
    return () => window.removeEventListener('keydown', onKey);
  }, [acknowledged, onClose]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(token);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      /* clipboard unavailable; user can still select-and-copy manually */
    }
  };

  return createPortal(
    <div className="cw-dialog-backdrop" role="presentation">
      <div
        className="cw-dialog cw-webhook-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="cw-webhook-token-title"
      >
        <header className="cw-webhook-dialog-head">
          <div className="cw-webhook-dialog-title-row">
            <span className="cw-webhook-dialog-icon">
              <Icon name="shield" size={16} />
            </span>
            <h2 id="cw-webhook-token-title">Webhook 토큰</h2>
          </div>
          <p className="cw-dialog-warn">
            이 토큰은 지금 한 번만 표시됩니다. 닫으면 다시 볼 수 없으니 안전한 곳에 보관하세요.
          </p>
        </header>

        <div className="cw-token-block">
          <code className="cw-token-value">{token}</code>
          <button
            type="button"
            className="cw-token-copy"
            onClick={copy}
            aria-label="토큰 복사"
          >
            <Icon name={copied ? 'check' : 'download'} size={14} />
            {copied ? '복사됨' : 'Copy'}
          </button>
        </div>

        {curlSample && (
          <details className="cw-token-curl">
            <summary>
              <Icon name="chevron-right" size={14} />
              호출 예시 (curl)
            </summary>
            <pre>{curlSample}</pre>
          </details>
        )}

        <label className="cw-webhook-ack">
          <input
            type="checkbox"
            checked={acknowledged}
            onChange={(e) => setAcknowledged(e.target.checked)}
          />
          <span>토큰을 안전한 곳에 보관했습니다.</span>
        </label>

        <footer className="cw-webhook-actions">
          <button
            ref={closeBtnRef}
            type="button"
            className="cw-btn-primary"
            onClick={onClose}
            disabled={!acknowledged}
          >
            완료
          </button>
        </footer>
      </div>
    </div>,
    document.body,
  );
}
