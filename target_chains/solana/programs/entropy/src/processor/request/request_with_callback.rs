use solana_program::{entrypoint::ProgramResult, program_error::ProgramError, pubkey::Pubkey};

use crate::accounts::CallbackMeta;

use super::{
    parse_bytes32, parse_callback_accounts, parse_callback_ix_data, parse_pubkey, parse_u32,
    process_request_with_callback_common,
};

pub fn process_request_with_callback(
    program_id: &Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let args = parse_request_with_callback_args(data)?;
    process_request_with_callback_common(
        program_id,
        accounts,
        args.provider,
        Some(args.user_randomness),
        args.compute_unit_limit,
        args.callback_accounts,
        args.callback_ix_data,
    )
}

struct RequestWithCallbackArgs {
    provider: Pubkey,
    user_randomness: [u8; 32],
    compute_unit_limit: u32,
    callback_accounts: Vec<CallbackMeta>,
    callback_ix_data: Vec<u8>,
}

fn parse_request_with_callback_args(data: &[u8]) -> Result<RequestWithCallbackArgs, ProgramError> {
    let (provider, mut offset) = parse_pubkey(data, 0)?;
    let (user_randomness, next_offset) = parse_bytes32(data, offset)?;
    offset = next_offset;
    let (compute_unit_limit, next_offset) = parse_u32(data, offset)?;
    offset = next_offset;

    let (callback_accounts, consumed) = parse_callback_accounts(&data[offset..])?;
    offset = offset
        .checked_add(consumed)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let (callback_ix_data, consumed) = parse_callback_ix_data(&data[offset..])?;
    offset = offset
        .checked_add(consumed)
        .ok_or(ProgramError::InvalidInstructionData)?;

    if offset != data.len() {
        return Err(ProgramError::InvalidInstructionData);
    }

    Ok(RequestWithCallbackArgs {
        provider,
        user_randomness,
        compute_unit_limit,
        callback_accounts,
        callback_ix_data,
    })
}
