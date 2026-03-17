export type SubtitleLanguage = {
  code: string;
  name: string;
  nativeName: string;
};

export const SUBTITLE_LANGUAGES: SubtitleLanguage[] = [
  { code: "alb", name: "Albanian", nativeName: "Shqip" },
  { code: "ara", name: "Arabic", nativeName: "\u0627\u0644\u0639\u0631\u0628\u064A\u0629" },
  { code: "arm", name: "Armenian", nativeName: "\u0540\u0561\u0575\u0565\u0580\u0565\u0576" },
  { code: "baq", name: "Basque", nativeName: "Euskara" },
  { code: "ben", name: "Bengali", nativeName: "\u09AC\u09BE\u0982\u09B2\u09BE" },
  { code: "bos", name: "Bosnian", nativeName: "Bosanski" },
  { code: "bul", name: "Bulgarian", nativeName: "\u0411\u044A\u043B\u0433\u0430\u0440\u0441\u043A\u0438" },
  { code: "cat", name: "Catalan", nativeName: "Catal\u00E0" },
  { code: "chi", name: "Chinese (Simplified)", nativeName: "\u7B80\u4F53\u4E2D\u6587" },
  { code: "zht", name: "Chinese (Traditional)", nativeName: "\u7E41\u9AD4\u4E2D\u6587" },
  { code: "hrv", name: "Croatian", nativeName: "Hrvatski" },
  { code: "cze", name: "Czech", nativeName: "\u010Ce\u0161tina" },
  { code: "dan", name: "Danish", nativeName: "Dansk" },
  { code: "dut", name: "Dutch", nativeName: "Nederlands" },
  { code: "eng", name: "English", nativeName: "English" },
  { code: "est", name: "Estonian", nativeName: "Eesti" },
  { code: "fin", name: "Finnish", nativeName: "Suomi" },
  { code: "fre", name: "French", nativeName: "Fran\u00E7ais" },
  { code: "geo", name: "Georgian", nativeName: "\u10E5\u10D0\u10E0\u10D7\u10E3\u10DA\u10D8" },
  { code: "ger", name: "German", nativeName: "Deutsch" },
  { code: "gre", name: "Greek", nativeName: "\u0395\u03BB\u03BB\u03B7\u03BD\u03B9\u03BA\u03AC" },
  { code: "heb", name: "Hebrew", nativeName: "\u05E2\u05D1\u05E8\u05D9\u05EA" },
  { code: "hin", name: "Hindi", nativeName: "\u0939\u093F\u0928\u094D\u0926\u0940" },
  { code: "hun", name: "Hungarian", nativeName: "Magyar" },
  { code: "ice", name: "Icelandic", nativeName: "\u00CDslenska" },
  { code: "ind", name: "Indonesian", nativeName: "Bahasa Indonesia" },
  { code: "ita", name: "Italian", nativeName: "Italiano" },
  { code: "jpn", name: "Japanese", nativeName: "\u65E5\u672C\u8A9E" },
  { code: "kor", name: "Korean", nativeName: "\uD55C\uAD6D\uC5B4" },
  { code: "lav", name: "Latvian", nativeName: "Latvie\u0161u" },
  { code: "lit", name: "Lithuanian", nativeName: "Lietuvi\u0173" },
  { code: "mac", name: "Macedonian", nativeName: "\u041C\u0430\u043A\u0435\u0434\u043E\u043D\u0441\u043A\u0438" },
  { code: "may", name: "Malay", nativeName: "Bahasa Melayu" },
  { code: "nor", name: "Norwegian", nativeName: "Norsk" },
  { code: "per", name: "Persian", nativeName: "\u0641\u0627\u0631\u0633\u06CC" },
  { code: "pol", name: "Polish", nativeName: "Polski" },
  { code: "por", name: "Portuguese", nativeName: "Portugu\u00EAs" },
  { code: "pob", name: "Portuguese (Brazilian)", nativeName: "Portugu\u00EAs (Brasil)" },
  { code: "rum", name: "Romanian", nativeName: "Rom\u00E2n\u0103" },
  { code: "rus", name: "Russian", nativeName: "\u0420\u0443\u0441\u0441\u043A\u0438\u0439" },
  { code: "scc", name: "Serbian", nativeName: "\u0421\u0440\u043F\u0441\u043A\u0438" },
  { code: "sin", name: "Sinhala", nativeName: "\u0DC3\u0DD2\u0D82\u0DC4\u0DBD" },
  { code: "slo", name: "Slovak", nativeName: "Sloven\u010Dina" },
  { code: "slv", name: "Slovenian", nativeName: "Sloven\u0161\u010Dina" },
  { code: "spa", name: "Spanish", nativeName: "Espa\u00F1ol" },
  { code: "swe", name: "Swedish", nativeName: "Svenska" },
  { code: "tha", name: "Thai", nativeName: "\u0E44\u0E17\u0E22" },
  { code: "tur", name: "Turkish", nativeName: "T\u00FCrk\u00E7e" },
  { code: "ukr", name: "Ukrainian", nativeName: "\u0423\u043A\u0440\u0430\u0457\u043D\u0441\u044C\u043A\u0430" },
  { code: "urd", name: "Urdu", nativeName: "\u0627\u0631\u062F\u0648" },
  { code: "vie", name: "Vietnamese", nativeName: "Ti\u1EBFng Vi\u1EC7t" },
];

const languageByCode = new Map<string, SubtitleLanguage>();
for (const lang of SUBTITLE_LANGUAGES) {
  languageByCode.set(lang.code, lang);
}

export function getSubtitleLanguage(code: string): SubtitleLanguage | undefined {
  return languageByCode.get(code);
}
