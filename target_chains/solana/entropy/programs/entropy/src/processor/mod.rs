mod initialize;
mod register_provider;
mod request;
mod reveal_with_callback;

use bytemuck::{try_from_bytes, Pod};
use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, program_error::ProgramError,
    pubkey::Pubkey,
};

use self::{
    initialize::process_initialize,
    register_provider::process_register_provider,
    request::{process_request, process_request_with_callback},
    reveal_with_callback::process_reveal_with_callback,
};
use crate::{error::EntropyError, instruction::EntropyInstruction};

pub(crate) fn parse_args<T: Pod>(data: &[u8]) -> Result<&T, ProgramError> {
    if data.len() != core::mem::size_of::<T>() {
        return Err(ProgramError::InvalidInstructionData);
    }

    try_from_bytes::<T>(data).map_err(|_| ProgramError::InvalidInstructionData)
}

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
        EntropyInstruction::Request => process_request(program_id, accounts, payload),
        EntropyInstruction::RequestWithCallback => {
            process_request_with_callback(program_id, accounts, payload)
        }
        EntropyInstruction::Reveal => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::RevealWithCallback => {
            process_reveal_with_callback(program_id, accounts, payload)
        }
        EntropyInstruction::AdvanceProviderCommitment => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::UpdateProviderConfig => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::WithdrawProviderFees => Err(EntropyError::NotImplemented.into()),
        EntropyInstruction::Governance => Err(EntropyError::NotImplemented.into()),
    }
}
