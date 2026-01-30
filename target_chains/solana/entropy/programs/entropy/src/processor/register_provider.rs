#[allow(deprecated)]
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_program,
};

use crate::{
    accounts::Provider,
    constants::PROVIDER_SEED,
    discriminator::provider_discriminator,
    error::EntropyError,
    instruction::RegisterProviderArgs,
    pda::{provider_pda, provider_vault_pda},
    pda_loader::{init_pda_mut, load_account_mut},
    processor::parse_args,
    vault::init_vault_pda,
};

pub fn process_register_provider(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let args = parse_args::<RegisterProviderArgs>(data)?;

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
    let system_program_account = next_account_info(&mut account_info_iter)?;

    if !provider_authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    if !provider_authority.is_writable
        || !provider_account.is_writable
        || !provider_vault.is_writable
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

    let (expected_vault, _vault_bump) = provider_vault_pda(program_id, provider_authority.key);
    if provider_vault.key != &expected_vault {
        return Err(EntropyError::InvalidPda.into());
    }

    let mut provider = if provider_account.owner == &system_program::ID {
        init_pda_mut::<Provider>(
            program_id,
            provider_authority,
            provider_account,
            system_program_account,
            &[
                PROVIDER_SEED,
                provider_authority.key.as_ref(),
                &[provider_bump],
            ],
            Provider::LEN,
        )?
    } else {
        let provider = load_account_mut::<Provider>(provider_account, program_id)?;
        if provider.provider_authority != provider_authority.key.to_bytes() {
            return Err(EntropyError::InvalidAccount.into());
        }
        provider
    };

    init_vault_pda(provider_authority, provider_vault, system_program_account)?;

    provider.discriminator = provider_discriminator();
    provider.provider_authority = provider_authority.key.to_bytes();

    provider.fee_lamports = args.fee_lamports;
    provider.original_commitment = args.commitment;
    provider.original_commitment_sequence_number = provider.sequence_number;
    provider.current_commitment = args.commitment;
    provider.current_commitment_sequence_number = provider.sequence_number;
    provider.commitment_metadata_len = args.commitment_metadata_len;
    provider.commitment_metadata = args.commitment_metadata;
    provider.uri_len = args.uri_len;
    provider.uri = args.uri;

    provider.end_sequence_number = provider
        .sequence_number
        .checked_add(args.chain_length)
        .ok_or(ProgramError::InvalidArgument)?;
    provider.sequence_number += 1;

    provider.bump = provider_bump;

    Ok(())
}
