# Comb — Honeyland Recovery Tool

An unofficial, community-built tool to recover NFTs frozen by Honeyland's on-chain staking programs, for use while the official HiveMind frontend is unavailable.

## What it does

Scans your wallet for frozen Honeyland NFTs, classifies each one by which on-chain program actually holds it, and lets you unfreeze anything that's recoverable:

- **Honeyland Core (classic NFTs)** — working
- **Generations (Metaplex Core assets)** — working
- **Legacy staking program** — confirmed permanently non-functional (its on-chain bytecode has been deleted); NFTs stuck here cannot be recovered by any client-side tool, this one included. See the in-app note for details.

## Disclaimer

**This is an unofficial, community-built tool.** It is not affiliated with, endorsed by, or supported by Honeyland or its team in any way.

It is provided **as-is, with no warranty of any kind**, express or implied. Use it entirely at your own risk.

Everything runs client-side, in your own browser. Your private key (if you choose to paste one) or wallet connection is never transmitted anywhere — all transaction signing happens locally on your device. You can verify this yourself by reading the source, since it's a single plain HTML file with no build step or external backend.

**You are solely responsible** for reviewing what this tool does before using it, and for any transaction you approve. **The creator(s) accept no liability** for any loss of funds, NFTs, or other damages arising from use of this tool, direct or indirect.

## Getting an RPC endpoint (required)

Scanning your wallet needs a Solana RPC provider that supports the DAS API (`getAssetsByOwner`, `getAsset`) — this is an extension not offered by plain public RPC. **Helius** is free and quick:

1. Go to [helius.dev](https://www.helius.dev)
2. Sign up (email only, no credit card needed for the free tier)
3. Your dashboard shows an RPC URL that already has your API key built in, looking like `https://mainnet.helius-rpc.com/?api-key=xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`
4. Copy that entire URL and paste it into the app's RPC field

The free tier's request limits are generous enough for scanning and recovering a normal-sized wallet.

## Usage

1. Open `index.html` (hosted via GitHub Pages, or run locally) inside your wallet's built-in browser (Solflare, Phantom, etc.) — this is required for the wallet-connect option to work. Alternatively, paste a private key directly (used only in-browser, never transmitted).
2. Provide your own Solana RPC endpoint (a Helius endpoint or similar with DAS API support is required for scanning).
3. Connect your wallet or load a key.
4. Scan, review, select, and unfreeze.

## Support

If this tool helped you and you'd like to say thanks, a donation address is included in the app footer — entirely optional, never required.
