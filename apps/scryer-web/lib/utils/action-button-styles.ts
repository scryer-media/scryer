export const boxedActionButtonBaseClass =
  "h-9 w-9 border shadow-[0_1px_2px_rgba(15,23,42,0.05),inset_0_1px_0_rgba(255,255,255,0.8)] transition-[color,background-color,border-color,box-shadow,transform] hover:-translate-y-px hover:shadow-[0_10px_20px_rgba(15,23,42,0.08),inset_0_1px_0_rgba(255,255,255,0.85)] dark:shadow-none dark:hover:translate-y-0 dark:hover:shadow-none";

export const boxedActionButtonToneClass = {
  auto:
    "border-emerald-200 bg-emerald-50 text-emerald-700 hover:border-emerald-300 hover:bg-emerald-100 hover:text-emerald-800 dark:border-emerald-500/35 dark:bg-emerald-500/12 dark:text-emerald-200 dark:hover:border-emerald-400/45 dark:hover:bg-emerald-500/22 dark:hover:text-emerald-50",
  search:
    "border-sky-200 bg-sky-50 text-sky-700 hover:border-sky-300 hover:bg-sky-100 hover:text-sky-800 dark:border-sky-500/35 dark:bg-sky-500/12 dark:text-sky-200 dark:hover:border-sky-400/45 dark:hover:bg-sky-500/22 dark:hover:text-sky-50",
  edit:
    "border-sky-200 bg-sky-50 text-sky-700 hover:border-sky-300 hover:bg-sky-100 hover:text-sky-800 dark:border-sky-500/35 dark:bg-sky-500/12 dark:text-sky-200 dark:hover:border-sky-400/45 dark:hover:bg-sky-500/22 dark:hover:text-sky-50",
  reorder:
    "border-indigo-200 bg-indigo-50 text-indigo-700 hover:border-indigo-300 hover:bg-indigo-100 hover:text-indigo-800 dark:border-sky-500/35 dark:bg-sky-500/12 dark:text-sky-200 dark:hover:border-sky-400/45 dark:hover:bg-sky-500/22 dark:hover:text-sky-50",
  install:
    "border-emerald-200 bg-emerald-50 text-emerald-700 hover:border-emerald-300 hover:bg-emerald-100 hover:text-emerald-800 dark:border-emerald-500/35 dark:bg-emerald-500/12 dark:text-emerald-200 dark:hover:border-emerald-400/45 dark:hover:bg-emerald-500/22 dark:hover:text-emerald-50",
  upgrade:
    "border-amber-200 bg-amber-50 text-amber-700 hover:border-amber-300 hover:bg-amber-100 hover:text-amber-800 dark:border-amber-500/40 dark:bg-amber-500/14 dark:text-amber-200 dark:hover:border-amber-400/50 dark:hover:bg-amber-500/24 dark:hover:text-amber-50",
  enabled:
    "border-emerald-200 bg-emerald-50 text-emerald-700 hover:border-emerald-300 hover:bg-emerald-100 hover:text-emerald-800 dark:border-emerald-500/35 dark:bg-emerald-500/12 dark:text-emerald-200 dark:hover:border-emerald-400/45 dark:hover:bg-emerald-500/22 dark:hover:text-emerald-50",
  disabled:
    "border-rose-200 bg-rose-50 text-rose-700 hover:border-rose-300 hover:bg-rose-100 hover:text-rose-800 dark:border-rose-500/35 dark:bg-rose-500/12 dark:text-rose-200 dark:hover:border-rose-400/45 dark:hover:bg-rose-500/22 dark:hover:text-rose-50",
  delete:
    "border-red-200 bg-red-50 text-red-700 hover:border-red-300 hover:bg-red-100 hover:text-red-800 dark:border-red-500/40 dark:bg-red-500/14 dark:text-red-200 dark:hover:border-red-400/50 dark:hover:bg-red-500/26 dark:hover:text-red-50",
} as const;

export type BoxedActionButtonTone = keyof typeof boxedActionButtonToneClass;
