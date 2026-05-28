import i18n from 'i18next';
import LanguageDetector from 'i18next-browser-languagedetector';
import resourcesToBackend from 'i18next-resources-to-backend';
import { initReactI18next } from 'react-i18next';

import enCommon from './locales/en/common.json';
import koCommon from './locales/ko/common.json';

export { josa, type JosaPair } from './josa';

export const SUPPORTED_LANGUAGES = ['en', 'ko'] as const;
export type SupportedLanguage = (typeof SUPPORTED_LANGUAGES)[number];

export const LANGUAGE_STORAGE_KEY = 'cw-lang';
export const I18N_NAMESPACES = [
  'common',
  'auth',
  'project',
  'session',
  'files',
  'dialogs',
  'errors',
  'members',
] as const;

void i18n
  .use(
    resourcesToBackend(
      (lng: string, ns: string) => import(`./locales/${lng}/${ns}.json`),
    ),
  )
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    fallbackLng: 'en',
    supportedLngs: [...SUPPORTED_LANGUAGES],
    ns: ['common'],
    defaultNS: 'common',
    partialBundledLanguages: true,
    resources: {
      en: { common: enCommon },
      ko: { common: koCommon },
    },
    interpolation: {
      escapeValue: false,
    },
    detection: {
      order: ['localStorage', 'navigator'],
      lookupLocalStorage: LANGUAGE_STORAGE_KEY,
      caches: ['localStorage'],
    },
    returnNull: false,
  });

export default i18n;
