//! Transaction building and submission.

use crate::error::{BlockchainError, BlockchainResult};
#[cfg(test)]
use solana_sdk::signature::Keypair;
use solana_sdk::{
    hash::Hash, instruction::Instruction, pubkey::Pubkey, signature::Signature, signer::Signer,
    transaction::Transaction,
};
use std::fmt;

/// Builder for constructing Solana transactions.
pub struct TransactionBuilder {
    instructions: Vec<Instruction>,
    payer: Option<Pubkey>,
}

impl TransactionBuilder {
    /// Create a new transaction builder.
    pub fn new() -> Self {
        TransactionBuilder {
            instructions: Vec::new(),
            payer: None,
        }
    }

    /// Set the payer for the transaction.
    pub fn payer(mut self, payer: Pubkey) -> Self {
        self.payer = Some(payer);
        self
    }

    /// Add an instruction to the transaction.
    pub fn add_instruction(mut self, instruction: Instruction) -> Self {
        self.instructions.push(instruction);
        self
    }

    /// Add multiple instructions.
    pub fn add_instructions(mut self, instructions: Vec<Instruction>) -> Self {
        self.instructions.extend(instructions);
        self
    }

    /// Get the number of instructions.
    pub fn instruction_count(&self) -> usize {
        self.instructions.len()
    }

    /// Check if builder is empty.
    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    /// Clear all instructions.
    pub fn clear(&mut self) {
        self.instructions.clear();
    }

    /// Get the instructions (for inspection).
    pub fn get_instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    /// Build and sign a transaction.
    pub fn build_and_sign(
        self,
        recent_blockhash: [u8; 32],
        signers: &[&dyn Signer],
    ) -> BlockchainResult<Transaction> {
        if self.instructions.is_empty() {
            return Err(BlockchainError::TransactionError(
                "No instructions in transaction".to_string(),
            ));
        }

        if signers.is_empty() {
            return Err(BlockchainError::TransactionError(
                "No signers provided".to_string(),
            ));
        }

        let payer = self.payer.ok_or(BlockchainError::TransactionError(
            "No payer specified".to_string(),
        ))?;

        let transaction = Transaction::new_signed_with_payer(
            &self.instructions,
            Some(&payer),
            signers,
            Hash::new_from_array(recent_blockhash),
        );

        Ok(transaction)
    }

    /// Build transaction without signing (for simulation).
    pub fn build_unsigned(self, recent_blockhash: [u8; 32]) -> BlockchainResult<Transaction> {
        if self.instructions.is_empty() {
            return Err(BlockchainError::TransactionError(
                "No instructions in transaction".to_string(),
            ));
        }

        let payer = self.payer.ok_or(BlockchainError::TransactionError(
            "No payer specified".to_string(),
        ))?;

        let mut transaction = Transaction::new_with_payer(&self.instructions, Some(&payer));
        transaction.message.recent_blockhash = Hash::new_from_array(recent_blockhash);

        Ok(transaction)
    }

    /// Estimate transaction size.
    pub fn estimate_size(&self) -> usize {
        // Rough estimation: header + instructions + signatures
        // Actual size depends on serialization
        let header_size = 64; // Estimate for header
        let instruction_size: usize = self
            .instructions
            .iter()
            .map(|ix| 1 + 4 + ix.accounts.len() * 32 + ix.data.len())
            .sum();

        header_size + instruction_size
    }
}

impl Default for TransactionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TransactionBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransactionBuilder")
            .field("instructions", &self.instructions.len())
            .field("payer", &self.payer)
            .finish()
    }
}

/// Transaction submission result with metadata.
#[derive(Debug, Clone)]
pub struct SubmissionResult {
    pub signature: Signature,
    pub sent_at: chrono::DateTime<chrono::Utc>,
    pub estimated_cost: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_creation() {
        let builder = TransactionBuilder::new();
        assert!(builder.is_empty());
        assert_eq!(builder.instruction_count(), 0);
    }

    #[test]
    fn test_builder_with_payer() {
        let payer = Pubkey::new_unique();
        let builder = TransactionBuilder::new().payer(payer);
        assert_eq!(builder.payer, Some(payer));
    }

    #[test]
    fn test_builder_no_payer_error() {
        let builder = TransactionBuilder::new();
        let blockhash = [0u8; 32];

        let keypair = Keypair::new();
        let result = builder.build_and_sign(blockhash, &[&keypair]);

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_no_instructions_error() {
        let payer = Pubkey::new_unique();
        let builder = TransactionBuilder::new().payer(payer);
        let blockhash = [0u8; 32];

        let keypair = Keypair::new();
        let result = builder.build_and_sign(blockhash, &[&keypair]);

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_size_estimation() {
        let builder = TransactionBuilder::new();
        let size = builder.estimate_size();
        assert!(size > 0);
    }

    #[test]
    fn test_builder_clear() {
        let mut builder = TransactionBuilder::new();
        let instruction = Instruction::new_with_bytes(Pubkey::new_unique(), &[1, 2, 3], vec![]);
        builder = builder.add_instruction(instruction);
        assert_eq!(builder.instruction_count(), 1);

        builder.clear();
        assert_eq!(builder.instruction_count(), 0);
    }
}
