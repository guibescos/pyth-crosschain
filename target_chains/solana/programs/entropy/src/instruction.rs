use bytemuck::{Pod, Zeroable};

use crate::accounts::PubkeyBytes;

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

impl EntropyInstruction {
    pub fn parse(
        input: &[u8],
    ) -> Result<(EntropyInstruction, &[u8]), solana_program::program_error::ProgramError> {
        if input.is_empty() {
            return Err(solana_program::program_error::ProgramError::InvalidInstructionData);
        }
        let payload = &input[1..];
        let instruction = match input[0] {
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
