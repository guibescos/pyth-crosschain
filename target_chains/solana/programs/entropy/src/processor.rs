use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, pubkey::Pubkey};

use crate::{error::EntropyError, instruction::EntropyInstruction};

pub fn process_instruction(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let (instruction, _payload) = EntropyInstruction::parse(data)?;
    match instruction {
        EntropyInstruction::Initialize => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::RegisterProvider => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::Request => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::RequestWithCallback => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::Reveal => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::RevealWithCallback => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::AdvanceProviderCommitment => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::UpdateProviderConfig => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::WithdrawProviderFees => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::Governance => Err(EntropyError::NotImplemented.into()),
    }
}
