import type { ProviderAdapter } from "@ldm/contracts";
import { dashAdapter } from "./dash";
import { genericDownloadLinksAdapter } from "./generic-download-links";
import { genericHtml5VideoAdapter } from "./generic-html5-video";
import { hlsAdapter } from "./hls";
import { tiktokAdapter } from "./provider-tiktok";
import { xAdapter } from "./provider-x";
import { youtubeAdapter } from "./provider-youtube";

export const adapters: ProviderAdapter[] = [
  genericHtml5VideoAdapter,
  genericDownloadLinksAdapter,
  hlsAdapter,
  dashAdapter,
  youtubeAdapter,
  xAdapter,
  tiktokAdapter,
];
