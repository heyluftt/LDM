/**
 * The LDM mark (source: assets/logo/ldm-mark.svg) for the content script's overlay button and
 * the background's notifications. Kept as a string so it can be used as a `data:` URL and as
 * inline markup without a bundler step.
 */
const BRAND_MARK_SVG = [
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">',
  "<defs>",
  '<linearGradient id="t" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#16233a"/><stop offset="1" stop-color="#070b13"/></linearGradient>',
  '<linearGradient id="a" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#5cf59c"/><stop offset="1" stop-color="#15c364"/></linearGradient>',
  '<linearGradient id="c" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#8fa3bd"/><stop offset="1" stop-color="#5f7390"/></linearGradient>',
  "</defs>",
  '<rect x="16" y="16" width="480" height="480" rx="116" fill="url(#t)"/>',
  '<rect x="24.5" y="24.5" width="463" height="463" rx="108" fill="none" stroke="#fff" stroke-opacity=".12" stroke-width="3"/>',
  '<path d="M144 332v28a56 56 0 0 0 56 56h112a56 56 0 0 0 56-56v-28" fill="none" stroke="url(#c)" stroke-width="40" stroke-linecap="round" stroke-linejoin="round"/>',
  '<path d="M256 112v178" fill="none" stroke="url(#a)" stroke-width="58" stroke-linecap="round"/>',
  '<path d="M176 222l80 82 80-82" fill="none" stroke="url(#a)" stroke-width="58" stroke-linecap="round" stroke-linejoin="round"/>',
  "</svg>",
].join("");

export const BRAND_MARK_DATA_URL = `data:image/svg+xml;utf8,${encodeURIComponent(BRAND_MARK_SVG)}`;

/** Creates the mark as an inline `<svg>` element of `size` pixels. */
export function createBrandMarkElement(size: number): SVGSVGElement {
  const template = document.createElement("template");
  template.innerHTML = BRAND_MARK_SVG;
  const svg = template.content.firstElementChild as SVGSVGElement;
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("aria-hidden", "true");
  svg.style.cssText = `display:block;width:${size}px;height:${size}px;flex:0 0 ${size}px`;
  return svg;
}
