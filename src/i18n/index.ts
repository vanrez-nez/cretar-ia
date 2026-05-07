import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json";
import es from "./locales/es.json";

export const DEFAULT_LOCALE = "en";
export const SUPPORTED_LOCALES = ["en", "es"] as const;
export type SupportedLocale = (typeof SUPPORTED_LOCALES)[number];
export type AppLanguage = "system" | SupportedLocale;

const resources = {
  en: { translation: en },
  es: { translation: es },
};

export function resolveSystemLocale(language?: string): SupportedLocale {
  const candidate = language?.split("-")[0]?.toLowerCase();
  return isSupportedLocale(candidate) ? candidate : DEFAULT_LOCALE;
}

export function resolveAppLocale(language: AppLanguage | undefined | null): SupportedLocale {
  if (language && language !== "system" && isSupportedLocale(language)) {
    return language;
  }
  if (typeof navigator !== "undefined") {
    return resolveSystemLocale(navigator.language);
  }
  return DEFAULT_LOCALE;
}

function isSupportedLocale(value: string | undefined): value is SupportedLocale {
  return SUPPORTED_LOCALES.includes(value as SupportedLocale);
}

i18n.use(initReactI18next).init({
  resources,
  lng: resolveAppLocale("system"),
  fallbackLng: DEFAULT_LOCALE,
  interpolation: {
    escapeValue: false,
  },
});

export default i18n;
