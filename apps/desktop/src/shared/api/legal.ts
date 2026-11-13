import { invoke } from "./invoke";

export type LegalStatus = {
  legalTermsVersion: string;
  acceptedLegalVersion?: string;
  acceptedAt?: string;
  accepted: boolean;
};

export function getLegalStatus(): Promise<LegalStatus> {
  return invoke<LegalStatus>("legal_status");
}

export function acceptLegalTerms(): Promise<LegalStatus> {
  return invoke<LegalStatus>("accept_legal_terms");
}
