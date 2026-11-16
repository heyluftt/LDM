import type { LegalStatus } from "../../shared/api/legal";
import { type LegalDocumentId, legalDocuments } from "../../shared/legal";
import { formatAcceptedAt } from "./settingsData";

export function LegalSettingsSection({
  legalStatus,
  onSelectDocument,
}: {
  legalStatus: LegalStatus | null;
  onSelectDocument: (documentId: LegalDocumentId) => void;
}) {
  return (
    <section className="panel">
      <div className="panel-header">
        <div>
          <h2>Legal</h2>
        </div>
      </div>
      <div className="settings-list">
        <div className="legal-status-grid">
          <span>Current version</span>
          <strong>{legalStatus?.legalTermsVersion ?? "Unknown"}</strong>
          <span>Accepted version</span>
          <strong>{legalStatus?.acceptedLegalVersion ?? "Not accepted"}</strong>
          <span>Accepted date</span>
          <strong>{formatAcceptedAt(legalStatus?.acceptedAt)}</strong>
        </div>
        <div className="legal-doc-actions">
          {legalDocuments.map((document) => (
            <button
              className="secondary-action"
              type="button"
              key={document.id}
              onClick={() => onSelectDocument(document.id)}
            >
              View {document.title}
            </button>
          ))}
        </div>
      </div>
    </section>
  );
}
