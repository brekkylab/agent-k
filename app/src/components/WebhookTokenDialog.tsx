import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Icon } from '@/components/Icon';

export function WebhookTokenDialog({
  token,
  curlSample,
  onClose,
  onDiscard,
  discardLabel,
  discardArmed = false,
}: {
  token: string;
  curlSample?: string;
  onClose: () => void;
  onDiscard?: () => void;
  discardLabel?: string;
  discardArmed?: boolean;
}) {
  const { t } = useTranslation('automation');
  const [acknowledged, setAcknowledged] = useState(false);
  const [copied, setCopied] = useState(false);
  const [curlCopied, setCurlCopied] = useState(false);
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);

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
            <h2 id="cw-webhook-token-title">{t('webhook_dialog.title')}</h2>
          </div>
          <p className="cw-dialog-warn">
            {t('webhook_dialog.warn')}
          </p>
        </header>

        <div className="cw-token-block">
          <code className="cw-token-value">{token}</code>
          <button
            type="button"
            className="cw-token-copy"
            onClick={copy}
            aria-label={t('webhook_dialog.copy_token_aria')}
          >
            <Icon name={copied ? 'check' : 'copy'} size={14} />
            {copied ? t('webhook_dialog.copied') : t('webhook_dialog.copy')}
          </button>
        </div>

        {curlSample && (
          <details className="cw-token-curl">
            <summary>
              <Icon name="chevron-right" size={14} />
              {t('webhook_dialog.curl_summary')}
            </summary>
            <div className="cw-token-curl-body">
              <pre>{curlSample}</pre>
              <button
                type="button"
                className="cw-token-curl-copy"
                onClick={copyCurl}
                aria-label={t('webhook_dialog.copy_curl_aria')}
              >
                <Icon name={curlCopied ? 'check' : 'copy'} size={12} />
                {curlCopied ? t('webhook_dialog.copied') : t('webhook_dialog.copy')}
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
          <span>{t('webhook_dialog.ack')}</span>
        </label>

        <footer className="cw-webhook-actions">
          {onDiscard && (
            <button
              type="button"
              className={`cw-btn-secondary cw-btn-destructive${discardArmed ? ' is-armed' : ''}`}
              onClick={onDiscard}
            >
              {discardLabel ?? t('webhook_dialog.discard_default')}
            </button>
          )}
          <button
            ref={closeBtnRef}
            type="button"
            className="cw-btn-primary"
            onClick={onClose}
            disabled={!acknowledged}
          >
            {t('webhook_dialog.done')}
          </button>
        </footer>
      </div>
    </div>,
    document.body,
  );
}
