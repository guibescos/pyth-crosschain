use bytemuck::{from_bytes_mut, try_from_bytes};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_instruction,
    system_program,
    sysvar::{rent::Rent, Sysvar},
};

use crate::{
    accounts::{Config, Provider},
    constants::{PROVIDER_SEED, PROVIDER_VAULT_SEED},
    discriminator::{config_discriminator, provider_discriminator},
    error::EntropyError,
    instruction::RegisterProviderArgs,
    pda::{config_pda, provider_pda, provider_vault_pda},
};

pub fn process_register_provider(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let args = parse_register_provider_args(data)?;

    if args.chain_length == 0 {
        return Err(ProgramError::InvalidArgument);
    }

    if (args.commitment_metadata_len as usize) > crate::constants::COMMITMENT_METADATA_LEN
        || (args.uri_len as usize) > crate::constants::URI_LEN
    {
        return Err(ProgramError::InvalidInstructionData);
    }

    let mut account_info_iter = accounts.iter();
    let provider_authority = next_account_info(&mut account_info_iter)?;
    let provider_account = next_account_info(&mut account_info_iter)?;
    let provider_vault = next_account_info(&mut account_info_iter)?;
    let config_account = next_account_info(&mut account_info_iter)?;
    let system_program_account = next_account_info(&mut account_info_iter)?;

    if !provider_authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    if !provider_authority.is_writable
        || !provider_account.is_writable
        || !provider_vault.is_writable
        || !config_account.is_writable
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    if system_program_account.key != &system_program::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    let (expected_provider, provider_bump) = provider_pda(program_id, provider_authority.key);
    if provider_account.key != &expected_provider {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_vault, vault_bump) = provider_vault_pda(program_id, provider_authority.key);
    if provider_vault.key != &expected_vault {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_config, _config_bump) = config_pda(program_id);
    if config_account.key != &expected_config {
        return Err(EntropyError::InvalidPda.into());
    }

    if config_account.owner != program_id || config_account.data_len() != Config::LEN {
        return Err(EntropyError::InvalidAccount.into());
    }

    let config_data = config_account.data.borrow();
    let config = try_from_bytes::<Config>(&config_data)
        .map_err(|_| ProgramError::InvalidAccountData)?;
    if config.discriminator != config_discriminator() {
        return Err(EntropyError::InvalidAccount.into());
    }

    let mut provider_created = false;
    if provider_account.owner == &system_program::ID && provider_account.data_len() == 0 {
        if provider_account.lamports() != 0 {
            return Err(EntropyError::InvalidAccount.into());
        }
        let rent = Rent::get()?;
        let provider_lamports = rent.minimum_balance(Provider::LEN);
        let create_provider_ix = system_instruction::create_account(
            provider_authority.key,
            provider_account.key,
            provider_lamports,
            Provider::LEN as u64,
            program_id,
        );
        invoke_signed(
            &create_provider_ix,
            &[
                provider_authority.clone(),
                provider_account.clone(),
                system_program_account.clone(),
            ],
            &[&[
                PROVIDER_SEED,
                provider_authority.key.as_ref(),
                &[provider_bump],
            ]],
        )?;
        provider_created = true;
    } else if provider_account.owner != program_id || provider_account.data_len() != Provider::LEN {
        return Err(EntropyError::InvalidAccount.into());
    }

    if provider_vault.owner == &system_program::ID
        && provider_vault.data_len() == 0
        && provider_vault.lamports() == 0
    {
        let rent = Rent::get()?;
        let vault_lamports = rent.minimum_balance(0);
        let create_vault_ix = system_instruction::create_account(
            provider_authority.key,
            provider_vault.key,
            vault_lamports,
            0,
            &system_program::ID,
        );
        invoke_signed(
            &create_vault_ix,
            &[
                provider_authority.clone(),
                provider_vault.clone(),
                system_program_account.clone(),
            ],
            &[&[
                PROVIDER_VAULT_SEED,
                provider_authority.key.as_ref(),
                &[vault_bump],
            ]],
        )?;
    } else if provider_vault.owner != &system_program::ID || provider_vault.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let mut provider_data = provider_account.data.borrow_mut();
    let provider = from_bytes_mut::<Provider>(&mut provider_data);
    if !provider_created {
        if provider.discriminator != provider_discriminator() {
            return Err(EntropyError::InvalidAccount.into());
        }
        if provider.provider_authority != provider_authority.key.to_bytes() {
            return Err(EntropyError::InvalidAccount.into());
        }
    }

    let end_sequence_number = provider
        .sequence_number
        .checked_add(args.chain_length)
        .ok_or(ProgramError::InvalidArgument)?;

    provider.discriminator = provider_discriminator();
    provider.provider_authority = provider_authority.key.to_bytes();
    provider.fee_lamports = args.fee_lamports;
    provider.original_commitment = args.commitment;
    provider.original_commitment_sequence_number = provider.sequence_number;
    provider.commitment_metadata_len = args.commitment_metadata_len;
    provider.commitment_metadata = args.commitment_metadata;
    provider.uri_len = args.uri_len;
    provider.uri = args.uri;
    provider._padding0 = [0u8; 4];
    provider.end_sequence_number = end_sequence_number;
    provider.sequence_number = provider.sequence_number + 1;
    provider.current_commitment = args.commitment;
    provider.current_commitment_sequence_number = provider.sequence_number - 1;
    provider.bump = provider_bump;
    provider._padding1 = [0u8; 7];

    Ok(())
}

fn parse_register_provider_args(data: &[u8]) -> Result<&RegisterProviderArgs, ProgramError> {
    if data.len() != core::mem::size_of::<RegisterProviderArgs>() {
        return Err(ProgramError::InvalidInstructionData);
    }

    try_from_bytes::<RegisterProviderArgs>(data)
        .map_err(|_| ProgramError::InvalidInstructionData)
}
