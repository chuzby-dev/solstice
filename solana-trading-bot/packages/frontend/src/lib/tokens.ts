// Mirrors the backend's default TOKEN_ALLOWLIST (.env.example). Phase 1 keeps this a
// static list in the GUI; a later phase can expose it via a settings endpoint instead.
export const TOKEN_ALLOWLIST = [
  { mint: "So11111111111111111111111111111111111111", symbol: "SOL" },
  { mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", symbol: "USDC" },
] as const;
