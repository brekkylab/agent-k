import { useTranslation } from 'react-i18next';
import { SUPPORTED_LANGUAGES, LANGUAGE_STORAGE_KEY, type SupportedLanguage } from '@/i18n';

const LABEL: Record<SupportedLanguage, string> = { en: 'EN', ko: '한' };

export function LanguageToggle() {
  const { t, i18n } = useTranslation('common');
  const lang = i18n.language as SupportedLanguage;

  function setLanguage(code: SupportedLanguage) {
    void i18n.changeLanguage(code);
    try { localStorage.setItem(LANGUAGE_STORAGE_KEY, code); } catch { /* noop */ }
  }

  return (
    <div className="cw-lang-toggle" role="group" aria-label={t('language.label')}>
      {SUPPORTED_LANGUAGES.map((code) => {
        const isActive = code === lang;
        return (
          <button
            key={code}
            type="button"
            className={`cw-lang-toggle__btn${isActive ? ' is-active' : ''}`}
            aria-pressed={isActive}
            onClick={() => { if (!isActive) setLanguage(code); }}
          >
            {LABEL[code]}
          </button>
        );
      })}
    </div>
  );
}
