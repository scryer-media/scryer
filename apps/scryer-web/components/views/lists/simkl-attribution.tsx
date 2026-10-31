import { simklItemUrl } from "@/lib/utils/simkl-links";

export function SimklAttribution({ itemKey }: { itemKey?: string }) {
  const href =
    itemKey === undefined ? "https://simkl.com" : simklItemUrl(itemKey);
  if (!href) return null;
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer noopener"
      className="inline-flex items-center gap-1 text-[11px] text-[var(--scry-accent-text)] hover:underline"
    >
      <img
        src="/plugin-logos/svg/simkl.svg"
        width={14}
        height={14}
        alt=""
        loading="lazy"
      />
      <span>Simkl</span>
    </a>
  );
}
