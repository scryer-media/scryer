import cronstrue from "cronstrue";

/**
 * The cronstrue locale for each interface language that cronstrue translates.
 * Hong Kong Chinese reads the traditional-script locale; languages missing
 * here, and any unknown code, describe in English.
 */
const CRONSTRUE_LOCALE_BY_UI_LANGUAGE: Record<string, string> = {
  eng: "en",
  spa: "es",
  fra: "fr",
  deu: "de",
  ita: "it",
  por: "pt_BR",
  kor: "ko",
  zho: "zh_CN",
  "zh-HK": "zh_TW",
  "zh-TW": "zh_TW",
  jpn: "ja",
  rus: "ru",
  nld: "nl",
};

const ENGLISH = "en";

// cronstrue ships English built in. Every other locale is a separate module
// that registers itself when imported, so each one is loaded on first use.
const LOCALE_LOADERS: Record<string, () => Promise<unknown>> = {
  es: () => import("cronstrue/locales/es.js"),
  fr: () => import("cronstrue/locales/fr.js"),
  de: () => import("cronstrue/locales/de.js"),
  it: () => import("cronstrue/locales/it.js"),
  pt_BR: () => import("cronstrue/locales/pt_BR.js"),
  ko: () => import("cronstrue/locales/ko.js"),
  zh_CN: () => import("cronstrue/locales/zh_CN.js"),
  zh_TW: () => import("cronstrue/locales/zh_TW.js"),
  ja: () => import("cronstrue/locales/ja.js"),
  ru: () => import("cronstrue/locales/ru.js"),
  nl: () => import("cronstrue/locales/nl.js"),
};

const loadedLocales = new Set<string>([ENGLISH]);
const pendingLoads = new Map<string, Promise<void>>();

/** The cronstrue locale for an interface language code, English when there is none. */
export function cronstrueLocaleFor(uiLanguage: string | null | undefined): string {
  if (!uiLanguage) {
    return ENGLISH;
  }
  return CRONSTRUE_LOCALE_BY_UI_LANGUAGE[uiLanguage] ?? ENGLISH;
}

export function isCronDescriptionLocaleLoaded(uiLanguage: string | null | undefined): boolean {
  return loadedLocales.has(cronstrueLocaleFor(uiLanguage));
}

/**
 * Loads the cronstrue locale for an interface language. Resolves once
 * descriptions can be given in it; a failed load leaves descriptions in English.
 */
export function loadCronDescriptionLocale(uiLanguage: string | null | undefined): Promise<void> {
  const locale = cronstrueLocaleFor(uiLanguage);
  if (loadedLocales.has(locale)) {
    return Promise.resolve();
  }
  const pending = pendingLoads.get(locale);
  if (pending) {
    return pending;
  }
  const loader = LOCALE_LOADERS[locale];
  if (!loader) {
    return Promise.resolve();
  }
  const load = loader()
    .then(() => {
      loadedLocales.add(locale);
    })
    .catch(() => undefined)
    .finally(() => {
      pendingLoads.delete(locale);
    });
  pendingLoads.set(locale, load);
  return load;
}

export type CronDescriptionOptions = {
  use24HourTimeFormat?: boolean;
};

/**
 * A human-readable sentence for a five-field cron expression, in the interface
 * language when its locale is loaded and in English otherwise. Null when the
 * expression is empty or cannot be read.
 */
export function describeCronExpression(
  expression: string,
  uiLanguage: string | null | undefined,
  options: CronDescriptionOptions = {},
): string | null {
  const trimmed = expression.trim();
  if (!trimmed) {
    return null;
  }
  const locale = cronstrueLocaleFor(uiLanguage);
  try {
    return cronstrue.toString(trimmed, {
      locale: loadedLocales.has(locale) ? locale : ENGLISH,
      use24HourTimeFormat: options.use24HourTimeFormat,
      throwExceptionOnParseError: true,
    });
  } catch {
    return null;
  }
}
