/**
 * The public key in the manifest. A browser derives the extension ID from it, so the extension
 * has the same ID on every machine and however it is loaded, and the desktop app can register
 * the native messaging host for it without asking. The matching private key is not part of the
 * repository and is only needed to pack a signed `.crx`.
 */
export const EXTENSION_PUBLIC_KEY =
  "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAkXWIwZv9123sofPyEW0S85VzrKnPHJjih4a9M01s7k98yJqo6obNRBexFbqKHPaH5FbzpVBbvwSvItklkSmARcwUD/dhqsbIU7QahUh6kKbBZAhMGnmyFCwV2qaZGCQrs1GufCXLR7YGji+BlrzO9O/2Mn9Lb/BxhpukYx/AECsAwQsjOjEqF1LtTDDmdvzdHfTPLq1arWbhvm2KEVzXIdniBUtJMuL8FiOpuIoo26utDl4ywgxUmrUOaQI4MbZn/a7wpJ64UmyHvIlpQk3DPxm+83qghLlRpaEikydbXKIAQYnQHI/+j7cs8GqWb+EYnNB+P5/gA4sFhvYKYUrEiwIDAQAB";

/**
 * The ID that follows from the key. `LDM_EXTENSION_ID` in `crates/ldm-types` is the same
 * string; tests on both sides keep the three in step.
 */
export const EXTENSION_ID = "dpfifoelihelgibggmccbedplhjpfmdc";
