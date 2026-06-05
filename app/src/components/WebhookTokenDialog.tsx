import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '@/components/Icon';
import { useDialogEscape } from '@/lib/useDialogEscape';

export function WebhookTokenDialog({
  token,
  curlSample,
  onClose,
  onDiscard,
  discardLabel = '생성 취소',
  discardArmed = false,
}: {
  token: string;
  curlSample?: string;
  onClose: () => void;
  onDiscard?: () => void;
  discardLabel?: string;
  discardArmed?: boolean;
}) {
  const [acknowledged, setAcknowledged] = useState(false);
  const [copied, setCopied] = useState(false);
  const [curlCopied, setCurlCopied] = useState(false);
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);

  // ESC closes only after the user has acknowledged the one-time secret. The
  // modal-stack hook treats `!acknowledged` as disabled so the press falls
  // through to any underlying dialog rather than being silently swallowed.
  useDialogEscape(onClose, { disabled: !acknowledged });
  useEffect(() => {
    closeBtnRef.current?.focus();
  }, []);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(token);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      /* clipboard unavailable; user can still select-and-copy manually */
    }
  };
  const copyCurl = async () => {
    if (!curlSample) return;
    try {
      await navigator.clipboard.writeText(curlSample);
      setCurlCopied(true);
      window.setTimeout(() => setCurlCopied(false), 1800);
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
            <Icon name={copied ? 'check' : 'copy'} size={14} />
            {copied ? '복사됨' : 'Copy'}
          </button>
        </div>

        {curlSample && (
          <details className="cw-token-curl">
            <summary>
              <Icon name="chevron-right" size={14} />
              호출 예시 (curl)
            </summary>
            <div className="cw-token-curl-body">
              <pre>{curlSample}</pre>
              <button
                type="button"
                className="cw-token-curl-copy"
                onClick={copyCurl}
                aria-label="curl 명령 복사"
              >
                <Icon name={curlCopied ? 'check' : 'copy'} size={12} />
                {curlCopied ? '복사됨' : 'Copy'}
              </button>
            </div>
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
          {onDiscard && (
            <button
              type="button"
              className={`cw-btn-secondary cw-btn-destructive${discardArmed ? ' is-armed' : ''}`}
              onClick={onDiscard}
            >
              {discardLabel}
            </button>
          )}
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
