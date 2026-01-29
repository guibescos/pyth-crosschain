use bytemuck::try_from_bytes;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program::set_return_data,
    program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::{
    constants::CALLBACK_NOT_NECESSARY,
    instruction::RequestArgs,
};

use super::{request_helper, RequestAccounts, RequestCallbackConfig};

pub fn process_request(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let args = parse_request_args(data)?;

    if args.use_blockhash > 1 {
        return Err(ProgramError::InvalidInstructionData);
    }

    let mut account_info_iter = accounts.iter();
    let requester_signer = next_account_info(&mut account_info_iter)?;
    let payer = next_account_info(&mut account_info_iter)?;
    let requester_program = next_account_info(&mut account_info_iter)?;
    let request_account = next_account_info(&mut account_info_iter)?;
    let provider_account = next_account_info(&mut account_info_iter)?;
    let provider_vault = next_account_info(&mut account_info_iter)?;
    let config_account = next_account_info(&mut account_info_iter)?;
    let pyth_fee_vault = next_account_info(&mut account_info_iter)?;
    let system_program_account = next_account_info(&mut account_info_iter)?;

    let sequence_number = request_helper(
        program_id,
        RequestAccounts {
            requester_signer,
            payer,
            requester_program,
            request_account,
            provider_account,
            provider_vault,
            config_account,
            pyth_fee_vault,
            system_program_account,
        },
        args,
        RequestCallbackConfig {
            callback_program_id: Pubkey::default(),
            callback_accounts: &[],
            callback_ix_data: &[],
            callback_status: CALLBACK_NOT_NECESSARY,
            compute_unit_limit: 0,
        },
    )?;

    // Return the assigned sequence number for CPI callers.
    set_return_data(&sequence_number.to_le_bytes());

    Ok(())
}

fn parse_request_args(data: &[u8]) -> Result<&RequestArgs, ProgramError> {
    if data.len() != core::mem::size_of::<RequestArgs>() {
        return Err(ProgramError::InvalidInstructionData);
    }

    try_from_bytes::<RequestArgs>(data).map_err(|_| ProgramError::InvalidInstructionData)
}
