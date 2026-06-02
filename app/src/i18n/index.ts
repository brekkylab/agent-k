import i18n from 'i18next';
import LanguageDetector from 'i18next-browser-languagedetector';
import resourcesToBackend from 'i18next-resources-to-backend';
import { initReactI18next } from 'react-i18next';

export { josa, localizedNoun, type JosaPair } from './josa';

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
  'automation',
] as const;

// All namespaces are loaded lazily via Vite chunks. Each route declares the
// ns it needs via `loadNs()` in its TanStack Router loader, so the relevant
// chunks are awaited before the route mounts — no `useTranslation` call
// leaks to the outer Suspense boundary and triggers a blank flash. The
// `<Suspense fallback={null}>` in `main.tsx` is a safety net for the rare
// case where a component reaches for an ns that no ancestor loader covers.
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
    defaultNS: 'common',
    interpolation: {
      escapeValue: false,
    },
    detection: {
      order: ['localStorage', 'navigator'],
      lookupLocalStorage: LANGUAGE_STORAGE_KEY,
      caches: ['localStorage'],
    },
    returnNull: false,
    returnEmptyString: false,
  });

export default i18n;
