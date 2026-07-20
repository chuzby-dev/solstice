//! Jupiter aggregator client: best-route quotes across every Solana DEX,
//! via Jupiter's public Quote/Swap-Instructions API.

use crate::error::{DexError, DexResult};
use crate::traits::DexClient;
use crate::types::{Liquidity, PriceUpdate, Quote, QuoteRequest, RouteSegment, SwapRequest};
use async_trait::async_trait;
use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::warn;

/// Jupiter's on-chain program ID (the aggregator/router program that swaps
/// ultimately execute through).
pub const JUPITER_PROGRAM_ID: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

const DEFAULT_API_BASE: &str = "https://api.jup.ag/v6";

pub struct JupiterClient {
    http: reqwest::Client,
    api_base: String,
    program_id: Pubkey,
}

impl JupiterClient {
    pub fn new() -> DexResult<Self> {
        Self::with_api_base(DEFAULT_API_BASE)
    }

    pub fn with_api_base(api_base: impl Into<String>) -> DexResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        let program_id = Pubkey::from_str(JUPITER_PROGRAM_ID)
            .expect("JUPITER_PROGRAM_ID is a valid base58 pubkey");

        Ok(JupiterClient {
            http,
            api_base: api_base.into(),
            program_id,
        })
    }

    async fn fetch_quote(&self, request: &QuoteRequest) -> DexResult<JupiterQuoteResponse> {
        let url = format!(
            "{}/quote?inputMint={}&outputMint={}&amount={}&slippageBps={}",
            self.api_base,
            request.input_mint,
            request.output_mint,
            request.amount,
            request.slippage_bps
        );

        let response = self.http.get(&url).send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DexError::ApiError {
                dex: "Jupiter".to_string(),
                message: format!("quote request failed ({status}): {body}"),
            });
        }

        response
            .json::<JupiterQuoteResponse>()
            .await
            .map_err(|e| DexError::ParseError {
                dex: "Jupiter".to_string(),
                message: e.to_string(),
            })
    }
}

#[async_trait]
impl DexClient for JupiterClient {
    async fn get_quote(&self, request: &QuoteRequest) -> DexResult<Quote> {
        let response = self.fetch_quote(request).await?;
        response.into_quote()
    }

    async fn get_orderbook(&self, _market: &Pubkey) -> DexResult<solstice_core::types::OrderBook> {
        // Jupiter is a route aggregator, not a market with its own book.
        Err(DexError::NoQuote)
    }

    async fn get_liquidity(&self, _market: &Pubkey) -> DexResult<Liquidity> {
        Err(DexError::NoQuote)
    }

    async fn build_swap_instructions(
        &self,
        swap: &SwapRequest,
        quote: &Quote,
    ) -> DexResult<Vec<Instruction>> {
        let quote_response = self
            .fetch_quote(&QuoteRequest::new(
                swap.input_mint,
                swap.output_mint,
                swap.amount,
                swap.slippage_bps,
            ))
            .await?;

        // Sanity-check that re-fetching the quote for this swap still
        // roughly matches what the caller already has, to catch stale
        // quotes before spending a round trip building instructions for them.
        if quote_response.out_amount()? == 0 || quote.out_amount == 0 {
            return Err(DexError::NoQuote);
        }

        let body = SwapInstructionsRequest {
            quote_response: quote_response.raw,
            user_public_key: swap.payer.to_string(),
        };

        let response = self
            .http
            .post(format!("{}/swap-instructions", self.api_base))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(DexError::ApiError {
                dex: "Jupiter".to_string(),
                message: format!("swap-instructions request failed ({status}): {text}"),
            });
        }

        let parsed: SwapInstructionsResponse =
            response.json().await.map_err(|e| DexError::ParseError {
                dex: "Jupiter".to_string(),
                message: e.to_string(),
            })?;

        parsed.into_instructions()
    }

    async fn subscribe_prices(&self, markets: &[Pubkey]) -> mpsc::Receiver<PriceUpdate> {
        // Jupiter has no push feed; poll the quote endpoint on an interval
        // and treat each configured market as a mint to price against USDC.
        let (tx, rx) = mpsc::channel(256);
        let http = self.http.clone();
        let api_base = self.api_base.clone();
        let markets = markets.to_vec();

        tokio::spawn(async move {
            const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
            let Ok(usdc) = Pubkey::from_str(USDC_MINT) else {
                return;
            };

            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                if tx.is_closed() {
                    return;
                }

                for &mint in &markets {
                    let url = format!(
                        "{api_base}/quote?inputMint={mint}&outputMint={usdc}&amount=1000000000&slippageBps=50"
                    );
                    let Ok(response) = http.get(&url).send().await else {
                        continue;
                    };
                    let Ok(parsed) = response.json::<JupiterQuoteResponse>().await else {
                        continue;
                    };
                    let (Ok(in_amount), Ok(out_amount)) = (parsed.in_amount(), parsed.out_amount())
                    else {
                        continue;
                    };
                    if in_amount == 0 {
                        continue;
                    }

                    let update = PriceUpdate {
                        dex: "Jupiter".to_string(),
                        market: mint,
                        price: out_amount as f64 / in_amount as f64,
                        timestamp: Utc::now(),
                    };
                    if tx.send(update).await.is_err() {
                        return;
                    }
                }
            }
        });

        rx
    }

    fn protocol_name(&self) -> &str {
        "Jupiter"
    }

    fn program_id(&self) -> &Pubkey {
        &self.program_id
    }
}

#[derive(Debug, Clone, Deserialize)]
struct JupiterRoutePlanStep {
    #[serde(rename = "swapInfo")]
    swap_info: JupiterSwapInfo,
}

#[derive(Debug, Clone, Deserialize)]
struct JupiterSwapInfo {
    label: String,
    #[serde(rename = "inputMint")]
    input_mint: String,
    #[serde(rename = "outputMint")]
    output_mint: String,
    #[serde(rename = "inAmount")]
    in_amount: String,
    #[serde(rename = "outAmount")]
    out_amount: String,
    #[serde(rename = "feeAmount")]
    fee_amount: String,
}

/// Raw Jupiter `/quote` response. Kept alongside the parsed fields so it can
/// be forwarded verbatim to `/swap-instructions`, which expects the exact
/// object `/quote` returned.
#[derive(Debug, Clone, Deserialize)]
struct JupiterQuoteResponse {
    #[serde(rename = "inAmount")]
    in_amount: String,
    #[serde(rename = "outAmount")]
    out_amount: String,
    #[serde(rename = "priceImpactPct")]
    price_impact_pct: String,
    #[serde(rename = "routePlan")]
    route_plan: Vec<JupiterRoutePlanStep>,

    #[serde(flatten)]
    raw: serde_json::Value,
}

impl JupiterQuoteResponse {
    fn in_amount(&self) -> DexResult<u64> {
        self.in_amount.parse().map_err(|_| DexError::ParseError {
            dex: "Jupiter".to_string(),
            message: format!("invalid inAmount: {}", self.in_amount),
        })
    }

    fn out_amount(&self) -> DexResult<u64> {
        self.out_amount.parse().map_err(|_| DexError::ParseError {
            dex: "Jupiter".to_string(),
            message: format!("invalid outAmount: {}", self.out_amount),
        })
    }

    fn into_quote(self) -> DexResult<Quote> {
        let in_amount = self.in_amount()?;
        let out_amount = self.out_amount()?;
        let price_impact: f64 = self.price_impact_pct.parse().unwrap_or(0.0);

        let mut route = Vec::with_capacity(self.route_plan.len());
        let mut fee_amount: u64 = 0;
        for step in &self.route_plan {
            let input_mint =
                Pubkey::from_str(&step.swap_info.input_mint).map_err(|_| DexError::ParseError {
                    dex: "Jupiter".to_string(),
                    message: format!("invalid route input mint: {}", step.swap_info.input_mint),
                })?;
            let output_mint = Pubkey::from_str(&step.swap_info.output_mint).map_err(|_| {
                DexError::ParseError {
                    dex: "Jupiter".to_string(),
                    message: format!("invalid route output mint: {}", step.swap_info.output_mint),
                }
            })?;
            let step_in: u64 = step.swap_info.in_amount.parse().unwrap_or(0);
            let step_out: u64 = step.swap_info.out_amount.parse().unwrap_or(0);
            fee_amount = fee_amount.saturating_add(step.swap_info.fee_amount.parse().unwrap_or(0));

            route.push(RouteSegment {
                dex: step.swap_info.label.clone(),
                input_mint,
                output_mint,
                input_amount: step_in,
                output_amount: step_out,
            });
        }

        let fee_bps = if in_amount == 0 {
            0
        } else {
            ((fee_amount as u128 * 10_000) / in_amount as u128).min(10_000) as u32
        };

        Ok(Quote {
            in_amount,
            out_amount,
            fee_amount,
            fee_bps,
            price_impact,
            liquidity: out_amount,
            route,
            timestamp: Utc::now(),
        })
    }
}

#[derive(Debug, Serialize)]
struct SwapInstructionsRequest {
    #[serde(rename = "quoteResponse")]
    quote_response: serde_json::Value,
    #[serde(rename = "userPublicKey")]
    user_public_key: String,
}

#[derive(Debug, Deserialize)]
struct JupiterInstruction {
    #[serde(rename = "programId")]
    program_id: String,
    accounts: Vec<JupiterAccountMeta>,
    data: String,
}

#[derive(Debug, Deserialize)]
struct JupiterAccountMeta {
    pubkey: String,
    #[serde(rename = "isSigner")]
    is_signer: bool,
    #[serde(rename = "isWritable")]
    is_writable: bool,
}

impl JupiterInstruction {
    fn into_instruction(self) -> DexResult<Instruction> {
        let program_id = Pubkey::from_str(&self.program_id).map_err(|_| DexError::ParseError {
            dex: "Jupiter".to_string(),
            message: format!("invalid instruction program id: {}", self.program_id),
        })?;

        let accounts = self
            .accounts
            .into_iter()
            .map(|a| {
                let pubkey = Pubkey::from_str(&a.pubkey).map_err(|_| DexError::ParseError {
                    dex: "Jupiter".to_string(),
                    message: format!("invalid account pubkey: {}", a.pubkey),
                })?;
                Ok(AccountMeta {
                    pubkey,
                    is_signer: a.is_signer,
                    is_writable: a.is_writable,
                })
            })
            .collect::<DexResult<Vec<_>>>()?;

        let data = base64::engine::general_purpose::STANDARD
            .decode(&self.data)
            .map_err(|e| DexError::ParseError {
                dex: "Jupiter".to_string(),
                message: format!("invalid instruction data base64: {e}"),
            })?;

        Ok(Instruction {
            program_id,
            accounts,
            data,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SwapInstructionsResponse {
    #[serde(rename = "computeBudgetInstructions", default)]
    compute_budget_instructions: Vec<JupiterInstruction>,
    #[serde(rename = "setupInstructions", default)]
    setup_instructions: Vec<JupiterInstruction>,
    #[serde(rename = "swapInstruction")]
    swap_instruction: JupiterInstruction,
    #[serde(rename = "cleanupInstruction")]
    cleanup_instruction: Option<JupiterInstruction>,
    #[serde(rename = "addressLookupTableAddresses", default)]
    address_lookup_table_addresses: Vec<String>,
}

impl SwapInstructionsResponse {
    fn into_instructions(self) -> DexResult<Vec<Instruction>> {
        if !self.address_lookup_table_addresses.is_empty() {
            warn!(
                "Jupiter swap uses {} address lookup table(s); building instructions without \
                 them means the resulting transaction may exceed the legacy size limit unless \
                 the caller assembles a versioned transaction with these ALTs",
                self.address_lookup_table_addresses.len()
            );
        }

        let mut instructions = Vec::new();
        for ix in self.compute_budget_instructions {
            instructions.push(ix.into_instruction()?);
        }
        for ix in self.setup_instructions {
            instructions.push(ix.into_instruction()?);
        }
        instructions.push(self.swap_instruction.into_instruction()?);
        if let Some(ix) = self.cleanup_instruction {
            instructions.push(ix.into_instruction()?);
        }

        Ok(instructions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_quote_json() -> serde_json::Value {
        serde_json::json!({
            "inputMint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "inAmount": "1000000",
            "outputMint": "So11111111111111111111111111111111111111112",
            "outAmount": "50000000",
            "otherAmountThreshold": "49750000",
            "swapMode": "ExactIn",
            "slippageBps": 50,
            "priceImpactPct": "0.0012",
            "routePlan": [
                {
                    "swapInfo": {
                        "ammKey": "amm1",
                        "label": "Raydium",
                        "inputMint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                        "outputMint": "So11111111111111111111111111111111111111112",
                        "inAmount": "1000000",
                        "outAmount": "50000000",
                        "feeAmount": "2500",
                        "feeMint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
                    },
                    "percent": 100
                }
            ],
            "contextSlot": 123456,
            "timeTaken": 0.01
        })
    }

    #[test]
    fn test_parse_quote_response() {
        let response: JupiterQuoteResponse = serde_json::from_value(sample_quote_json()).unwrap();
        let quote = response.into_quote().unwrap();

        assert_eq!(quote.in_amount, 1_000_000);
        assert_eq!(quote.out_amount, 50_000_000);
        assert_eq!(quote.fee_amount, 2_500);
        assert_eq!(quote.route.len(), 1);
        assert_eq!(quote.route[0].dex, "Raydium");
        assert!((quote.price_impact - 0.0012).abs() < 1e-9);
    }

    #[test]
    fn test_parse_quote_response_fee_bps() {
        let response: JupiterQuoteResponse = serde_json::from_value(sample_quote_json()).unwrap();
        let quote = response.into_quote().unwrap();

        // 2500 / 1_000_000 * 10_000 = 25 bps
        assert_eq!(quote.fee_bps, 25);
    }

    #[test]
    fn test_parse_quote_response_invalid_mint() {
        let mut json = sample_quote_json();
        json["routePlan"][0]["swapInfo"]["inputMint"] = serde_json::json!("not-a-pubkey");
        let response: JupiterQuoteResponse = serde_json::from_value(json).unwrap();

        assert!(response.into_quote().is_err());
    }

    #[test]
    fn test_swap_instructions_response_parsing() {
        let json = serde_json::json!({
            "computeBudgetInstructions": [],
            "setupInstructions": [],
            "swapInstruction": {
                "programId": JUPITER_PROGRAM_ID,
                "accounts": [
                    {
                        "pubkey": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                        "isSigner": false,
                        "isWritable": true
                    }
                ],
                "data": base64::engine::general_purpose::STANDARD.encode([1, 2, 3])
            },
            "cleanupInstruction": null,
            "addressLookupTableAddresses": []
        });

        let response: SwapInstructionsResponse = serde_json::from_value(json).unwrap();
        let instructions = response.into_instructions().unwrap();

        assert_eq!(instructions.len(), 1);
        assert_eq!(instructions[0].data, vec![1, 2, 3]);
        assert_eq!(instructions[0].accounts.len(), 1);
    }

    #[test]
    fn test_client_protocol_metadata() {
        let client = JupiterClient::new().unwrap();
        assert_eq!(client.protocol_name(), "Jupiter");
        assert_eq!(client.program_id().to_string(), JUPITER_PROGRAM_ID);
    }
}
