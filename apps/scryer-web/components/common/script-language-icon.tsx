import type { ScriptLanguageValue } from "@/lib/types/scripts";

const languageLogo = (file: string) => `${import.meta.env.BASE_URL}script-languages/${file}`;

const SCRIPT_LANGUAGE_LOGOS: Partial<Record<ScriptLanguageValue, string>> = {
  PYTHON: languageLogo("python.svg"),
  POWERSHELL: languageLogo("powershell.svg"),
  GO: languageLogo("go.svg"),
};

/** Shell and batch have no logo of their own, so each shows its prompt. */
const SCRIPT_LANGUAGE_PROMPTS: Partial<Record<ScriptLanguageValue, string>> = {
  SHELL: "$",
  BATCH: ">",
};

/** The mark of a script language: its logo, or the prompt its shell shows. */
export function ScriptLanguageIcon({ language }: { language: ScriptLanguageValue }) {
  const logo = SCRIPT_LANGUAGE_LOGOS[language];
  if (logo) {
    return <img src={logo} alt="" aria-hidden="true" className="h-3.5 w-auto shrink-0" />;
  }
  const prompt = SCRIPT_LANGUAGE_PROMPTS[language];
  if (!prompt) return null;
  return (
    <span
      aria-hidden="true"
      className="inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-[3px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] font-[var(--font-code)] text-[10px] font-bold leading-none text-[var(--scry-accent)]"
    >
      {prompt}
    </span>
  );
}
