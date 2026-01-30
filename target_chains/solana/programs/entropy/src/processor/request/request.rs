use bytemuck::try_from_bytes;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program::set_return_data,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_program,
};

use crate::{
    accounts::{Config, Provider},
    constants::REQUESTER_SIGNER_SEED,
    error::EntropyError,
    instruction::RequestArgs,
    pda::{config_pda, provider_pda, provider_vault_pda, pyth_fee_vault_pda},
    pda_loader::{load_account, load_account_mut},
    processor::request::request_helper,
};

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

    if !requester_signer.is_signer || !payer.is_signer || !request_account.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    if !payer.is_writable
        || !request_account.is_writable
        || !provider_account.is_writable
        || !provider_vault.is_writable
        || !pyth_fee_vault.is_writable
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    if system_program_account.key != &system_program::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    let requester_signer_seed = [REQUESTER_SIGNER_SEED, program_id.as_ref()];
    let (expected_requester_signer, _bump) =
        Pubkey::find_program_address(&requester_signer_seed, requester_program.key);
    if requester_signer.key != &expected_requester_signer {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_config, _config_bump) = config_pda(program_id);
    if config_account.key != &expected_config {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_pyth_fee_vault, _pyth_fee_vault_bump) = pyth_fee_vault_pda(program_id);
    if pyth_fee_vault.key != &expected_pyth_fee_vault {
        return Err(EntropyError::InvalidPda.into());
    }
    if pyth_fee_vault.owner != &system_program::ID || pyth_fee_vault.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    if request_account.owner != &system_program::ID || request_account.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let config = load_account::<Config>(config_account, program_id)?;
    let mut provider = load_account_mut::<Provider>(provider_account, program_id)?;
    let provider_authority = Pubkey::new_from_array(provider.provider_authority);
    let (expected_provider, _provider_bump) = provider_pda(program_id, &provider_authority);
    if provider_account.key != &expected_provider {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_provider_vault, _provider_vault_bump) =
        provider_vault_pda(program_id, &provider_authority);
    if provider_vault.key != &expected_provider_vault {
        return Err(EntropyError::InvalidPda.into());
    }
    if provider_vault.owner != &system_program::ID || provider_vault.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let sequence_number = request_helper(
        program_id,
        args,
        &config,
        &mut provider,
        payer,
        requester_program,
        request_account,
        provider_vault,
        pyth_fee_vault,
        system_program_account,
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
