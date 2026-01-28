use bytemuck::{Pod, Zeroable};

use crate::{
    accounts::PubkeyBytes,
    constants::{COMMITMENT_METADATA_LEN, URI_LEN},
};

#[repr(u8)]
pub enum EntropyInstruction {
    Initialize = 0,
    RegisterProvider = 1,
    Request = 2,
    RequestWithCallback = 3,
    Reveal = 4,
    RevealWithCallback = 5,
    AdvanceProviderCommitment = 6,
    UpdateProviderConfig = 7,
    WithdrawProviderFees = 8,
    Governance = 9,
}

pub const INSTRUCTION_DISCRIMINATOR_LEN: usize = 8;

impl EntropyInstruction {
    pub fn discriminator(self) -> [u8; 8] {
        (self as u64).to_le_bytes()
    }

    pub fn parse(
        input: &[u8],
    ) -> Result<(EntropyInstruction, &[u8]), solana_program::program_error::ProgramError> {
        if input.len() < INSTRUCTION_DISCRIMINATOR_LEN {
            return Err(solana_program::program_error::ProgramError::InvalidInstructionData);
        }
        let mut discriminator_bytes = [0u8; INSTRUCTION_DISCRIMINATOR_LEN];
        discriminator_bytes.copy_from_slice(&input[..INSTRUCTION_DISCRIMINATOR_LEN]);
        let discriminator = u64::from_le_bytes(discriminator_bytes);
        let payload = &input[INSTRUCTION_DISCRIMINATOR_LEN..];
        let instruction = match discriminator {
            0 => EntropyInstruction::Initialize,
            1 => EntropyInstruction::RegisterProvider,
            2 => EntropyInstruction::Request,
            3 => EntropyInstruction::RequestWithCallback,
            4 => EntropyInstruction::Reveal,
            5 => EntropyInstruction::RevealWithCallback,
            6 => EntropyInstruction::AdvanceProviderCommitment,
            7 => EntropyInstruction::UpdateProviderConfig,
            8 => EntropyInstruction::WithdrawProviderFees,
            9 => EntropyInstruction::Governance,
            _ => return Err(solana_program::program_error::ProgramError::InvalidInstructionData),
        };
        Ok((instruction, payload))
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct InitializeArgs {
    pub admin: PubkeyBytes,
    pub pyth_fee_lamports: u64,
    pub default_provider: PubkeyBytes,
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct RegisterProviderArgs {
    pub fee_lamports: u64,
    pub commitment: [u8; 32],
    pub commitment_metadata_len: u16,
    pub commitment_metadata: [u8; COMMITMENT_METADATA_LEN],
    pub _padding0: [u8; 6],
    pub chain_length: u64,
    pub uri_len: u16,
    pub uri: [u8; URI_LEN],
    pub _padding1: [u8; 6],
}
