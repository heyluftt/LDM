import { useId } from "react";

/**
 * The LDM mark (source: assets/logo/ldm-mark.svg) as an inline SVG, so the header and the
 * titlebar show the same artwork as the app icon. Gradient ids are made unique per instance
 * because the mark is rendered more than once per page.
 */
export function BrandMark({ size = 28, className }: { size?: number; className?: string }) {
  const id = useId().replace(/:/g, "");
  const tile = `ldm-tile-${id}`;
  const arrow = `ldm-arrow-${id}`;
  const cup = `ldm-cup-${id}`;
  const gloss = `ldm-gloss-${id}`;

  return (
    <svg
      aria-hidden="true"
      className={className}
      height={size}
      viewBox="0 0 512 512"
      width={size}
      xmlns="http://www.w3.org/2000/svg"
    >
      <title>LDM</title>
      <defs>
        <linearGradient id={tile} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="#16233a" />
          <stop offset="1" stopColor="#070b13" />
        </linearGradient>
        <linearGradient id={arrow} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="#5cf59c" />
          <stop offset="1" stopColor="#15c364" />
        </linearGradient>
        <linearGradient id={cup} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="#8fa3bd" />
          <stop offset="1" stopColor="#5f7390" />
        </linearGradient>
        <linearGradient id={gloss} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="#ffffff" stopOpacity="0.1" />
          <stop offset="1" stopColor="#ffffff" stopOpacity="0" />
        </linearGradient>
      </defs>
      <rect fill={`url(#${tile})`} height="480" rx="116" width="480" x="16" y="16" />
      <rect fill={`url(#${gloss})`} height="240" rx="116" width="480" x="16" y="16" />
      <rect
        fill="none"
        height="463"
        rx="108"
        stroke="#ffffff"
        strokeOpacity="0.12"
        strokeWidth="3"
        width="463"
        x="24.5"
        y="24.5"
      />
      <path
        d="M144 332 v28 a56 56 0 0 0 56 56 h112 a56 56 0 0 0 56 -56 v-28"
        fill="none"
        stroke={`url(#${cup})`}
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="40"
      />
      <path d="M256 112 v178" fill="none" stroke={`url(#${arrow})`} strokeLinecap="round" strokeWidth="58" />
      <path
        d="M176 222 l80 82 l80 -82"
        fill="none"
        stroke={`url(#${arrow})`}
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="58"
      />
    </svg>
  );
}
