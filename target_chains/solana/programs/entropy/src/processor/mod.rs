mod initialize;
mod register_provider;

use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, pubkey::Pubkey};

use crate::{error::EntropyError, instruction::EntropyInstruction};
use self::{initialize::process_initialize, register_provider::process_register_provider};

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let (instruction, payload) = EntropyInstruction::parse(data)?;
    match instruction {
        EntropyInstruction::Initialize => process_initialize(program_id, accounts, payload),
        EntropyInstruction::RegisterProvider => {
            process_register_provider(program_id, accounts, payload)
        }
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
