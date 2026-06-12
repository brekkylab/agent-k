import { useCallback, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { updateMe } from '@/api/auth';
import { ApiError } from '@/api/client';
import { Icon } from '@/components/Icon';
import { Select, type SelectOption } from '@/components/Select';
import { useDialogEscape } from '@/lib/useDialogEscape';
import { useAuthStore } from '@/stores/auth';
import { LANGUAGE_STORAGE_KEY, SUPPORTED_LANGUAGES, type SupportedLanguage } from '@/i18n';

const LANG_LABEL: Record<SupportedLanguage, string> = { en: 'English', ko: '한국어' };
const MAX_NAME_LEN = 100;
const TITLE_ID = 'cw-user-settings-title';
const ERROR_ID = 'cw-user-settings-error';

// Global-save modal for low-risk preferences (display name + language),
// persisted in a single PATCH /me. Security actions like password change are
// intentionally excluded — they belong in a dedicated, re-auth flow.
function UserSettingsDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation('dialogs');
  const { t: tCommon } = useTranslation('common');
  const queryClient = useQueryClient();
  const currentUser = useAuthStore((s) => s.currentUser);
  const setCurrentUser = useAuthStore((s) => s.setCurrentUser);

  // Capture the initial values once, at open time, via a lazy initializer so a
  // background ['me'] refetch calling setCurrentUser mid-edit can't shift the
  // dirty baseline.
  const [initialName] = useState(() => currentUser?.name ?? '');
  const [initialLang] = useState<SupportedLanguage>(() => (currentUser?.preferredLanguage ?? 'en') as SupportedLanguage);
  const [name, setName] = useState(initialName);
  const [lang, setLang] = useState<SupportedLanguage>(initialLang);
  const [error, setError] = useState<string | null>(null);

  const trimmed = name.trim();
  const dirty = trimmed !== initialName.trim() || lang !== initialLang;
  const valid = trimmed.length > 0 && trimmed.length <= MAX_NAME_LEN;

  const langOptions: SelectOption<SupportedLanguage>[] = SUPPORTED_LANGUAGES.map((code) => ({
    value: code,
    label: LANG_LABEL[code],
  }));

  const mutation = useMutation({
    mutationFn: () => updateMe({ displayName: trimmed, preferredLanguage: lang }),
    onSuccess: (user) => {
      // setCurrentUser also runs i18n.changeLanguage when user.preferredLanguage changed.
      setCurrentUser(user);
      queryClient.setQueryData(['me'], user);
      try { localStorage.setItem(LANGUAGE_STORAGE_KEY, user.preferredLanguage); } catch { /* private mode etc. — the UI already reflects the change */ }
      onClose();
    },
    onError: (err) =>
      setError(err instanceof ApiError ? `${err.status} — ${err.message}` : t('user_settings.errors.generic')),
  });

  const pending = mutation.isPending;
  useDialogEscape(onClose, { disabled: pending });
  const downOnBackdropRef = useRef(false);

  function submit() {
    if (!dirty || !valid || pending) return;
    mutation.mutate();
  }

  return createPortal(
    <div
      className="cw-dialog-backdrop"
      onMouseDown={(e) => { downOnBackdropRef.current = e.target === e.currentTarget; }}
      onClick={(e) => {
        const was = downOnBackdropRef.current;
        downOnBackdropRef.current = false;
        if (was && e.target === e.currentTarget && !pending) onClose();
      }}
    >
      <form
        className="cw-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={TITLE_ID}
        onSubmit={(e) => { e.preventDefault(); submit(); }}
      >
        <button type="button" className="cw-close" onClick={onClose} disabled={pending} aria-label={tCommon('actions.close')}>
          <Icon name="x" />
        </button>
        <h2 id={TITLE_ID} style={{ margin: '0 0 var(--cw-space-5)', fontSize: 18, letterSpacing: '-0.015em' }}>
          {t('user_settings.title')}
        </h2>

        <label className="cw-field">
          <span>{t('user_settings.fields.display_name')}</span>
          <input
            autoFocus
            value={name}
            maxLength={MAX_NAME_LEN}
            disabled={pending}
            aria-label={t('user_settings.fields.display_name')}
            aria-invalid={error !== null}
            aria-describedby={error ? ERROR_ID : undefined}
            onChange={(e) => { setName(e.target.value); if (error) setError(null); }}
          />
        </label>

        <div className="cw-field">
          <span>{t('user_settings.fields.language')}</span>
          <Select<SupportedLanguage>
            value={lang}
            onChange={(next) => { setLang(next); if (error) setError(null); }}
            options={langOptions}
            ariaLabel={t('user_settings.fields.language')}
            className="cw-field-select"
            triggerClassName="cw-field-select__trigger"
            renderTrigger={(opt) => (
              <span className="cw-field-select__value">
                <Icon name="globe" size={15} />
                {opt.label}
              </span>
            )}
          />
        </div>

        {error && (
          <div id={ERROR_ID} className="cw-dialog-warn" role="alert">
            <Icon name="x" size={12} /> {error}
          </div>
        )}

        <div style={{ display: 'flex', gap: 'var(--cw-space-3)', justifyContent: 'flex-end', marginTop: 'var(--cw-space-5)' }}>
          <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>
            {tCommon('actions.cancel')}
          </button>
          <button type="submit" className="cw-btn-primary" disabled={!dirty || !valid || pending}>
            {pending ? tCommon('state.saving') : tCommon('actions.save')}
          </button>
        </div>
      </form>
    </div>,
    document.body,
  );
}

// Hook so multiple entry points can share the open state + dialog rendering
// (mirrors the useNewProjectDialog pattern).
export function useUserSettingsDialog(): { open: () => void; dialog: ReactNode } {
  const [isOpen, setIsOpen] = useState(false);
  const open = useCallback(() => setIsOpen(true), []);
  const close = useCallback(() => setIsOpen(false), []);
  return { open, dialog: isOpen ? <UserSettingsDialog onClose={close} /> : null };
}
