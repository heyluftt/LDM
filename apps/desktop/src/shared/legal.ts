import privacyNotice from "../../../../docs/legal/PRIVACY_NOTICE.md?raw";
import termsOfUse from "../../../../docs/legal/TERMS_OF_USE.md?raw";
import thirdPartyNotices from "../../../../docs/legal/THIRD_PARTY_NOTICES.md?raw";

export type LegalDocumentId = "terms" | "privacy" | "thirdParty";

export type LegalDocument = {
  id: LegalDocumentId;
  title: string;
  body: string;
};

/**
 * The texts live in `docs/legal/` and are embedded at build time. The version the app asks the
 * user to accept comes from `docs/legal/VERSION` through the Rust side (`LegalStatus`).
 */
export const legalDocuments: LegalDocument[] = [
  legalDocument("terms", termsOfUse),
  legalDocument("privacy", privacyNotice),
  legalDocument("thirdParty", thirdPartyNotices),
];

export function legalDocumentById(id: LegalDocumentId): LegalDocument {
  return legalDocuments.find((document) => document.id === id) ?? legalDocuments[0];
}

/** Splits a Markdown document into its `# Title` line and the paragraphs below it. */
function legalDocument(id: LegalDocumentId, markdown: string): LegalDocument {
  const [firstLine = "", ...rest] = markdown.trim().split("\n");
  return {
    id,
    title: firstLine.replace(/^#\s+/u, "").trim(),
    body: rest.join("\n").trim(),
  };
}
