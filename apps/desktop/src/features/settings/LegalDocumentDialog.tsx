import type { LegalDocument } from "../../shared/legal";

export function LegalDocumentDialog({ document, onClose }: { document: LegalDocument | null; onClose: () => void }) {
  if (!document) {
    return null;
  }

  return (
    <div className="legal-backdrop" role="presentation">
      <section
        className="legal-dialog document-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="legal-doc-title"
      >
        <div className="legal-dialog-header">
          <h2 id="legal-doc-title">{document.title}</h2>
        </div>
        <div className="legal-document-body">
          {document.body.split("\n\n").map((paragraph) => (
            <p key={paragraph}>{paragraph}</p>
          ))}
        </div>
        <div className="legal-actions">
          <button className="primary-action" type="button" onClick={onClose}>
            Close
          </button>
        </div>
      </section>
    </div>
  );
}
