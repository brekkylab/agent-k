import { useTranslation } from 'react-i18next';

import { useLanguage } from '@/lib/useLanguage';
import { SUPPORTED_LANGUAGES, type SupportedLanguage } from '@/i18n';

const LABEL: Record<SupportedLanguage, string> = {
  en: 'EN',
  ko: '한',
};

export function LanguageToggle() {
  const { t } = useTranslation();
  const { lang, setLanguage, isUpdating } = useLanguage();

  return (
    <div
      className="cw-lang-toggle"
      role="group"
      aria-label={t('language.label')}
    >
      {SUPPORTED_LANGUAGES.map((code) => {
        const isActive = code === lang;
        return (
          <button
            key={code}
            type="button"
            className={`cw-lang-toggle__btn${isActive ? ' is-active' : ''}`}
            aria-pressed={isActive}
            disabled={isUpdating}
            onClick={() => {
              if (!isActive) setLanguage(code);
            }}
          >
            {LABEL[code]}
          </button>
        );
      })}
    </div>
  );
}
