use std::cell::RefMut;

use bytemuck::{from_bytes_mut, try_from_bytes};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    hash::hashv,
    program::{invoke, set_return_data},
    program_error::ProgramError,
    pubkey::Pubkey,
    system_instruction, system_program,
    sysvar::{clock::Clock, rent::Rent, Sysvar},
};

use crate::{
    accounts::{CallbackMeta, Config, Provider, Request},
    constants::{
        CALLBACK_IX_DATA_LEN, CALLBACK_NOT_NECESSARY, MAX_CALLBACK_ACCOUNTS, REQUESTER_SIGNER_SEED,
    },
    discriminator::{config_discriminator, provider_discriminator, request_discriminator},
    error::EntropyError,
    instruction::RequestArgs,
    pda::{config_pda, provider_pda, provider_vault_pda, pyth_fee_vault_pda},
};

use super::pda::{load_pda, load_pda_mut};

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

    let config = load_pda::<Config>(
        config_account,
        program_id,
        Config::LEN,
        config_discriminator(),
    )?;
    let mut provider = load_pda_mut::<Provider>(
        provider_account,
        program_id,
        Provider::LEN,
        provider_discriminator(),
    )?;
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

    // Assign a sequence number to the request
    let sequence_number = provider.sequence_number;
    if sequence_number >= provider.end_sequence_number {
        return Err(EntropyError::OutOfRandomness.into());
    }
    provider.sequence_number = provider
        .sequence_number
        .checked_add(1)
        .ok_or(ProgramError::InvalidArgument)?;

    // Calculate and transfer fees
    let provider_fee = provider.calculate_provider_fee(args.compute_unit_limit)?;
    if provider_fee > 0 {
        let transfer_ix = system_instruction::transfer(payer.key, provider_vault.key, provider_fee);
        invoke(
            &transfer_ix,
            &[
                payer.clone(),
                provider_vault.clone(),
                system_program_account.clone(),
            ],
        )?;
    }
    if config.pyth_fee_lamports > 0 {
        let transfer_ix =
            system_instruction::transfer(payer.key, pyth_fee_vault.key, config.pyth_fee_lamports);
        invoke(
            &transfer_ix,
            &[
                payer.clone(),
                pyth_fee_vault.clone(),
                system_program_account.clone(),
            ],
        )?;
    }

    let mut request = init_request_account_mut(
        program_id,
        payer,
        request_account,
        system_program_account,
        Request::LEN,
    )?;

    request.provider = provider.provider_authority;
    request.sequence_number = sequence_number;

    let num_hashes = sequence_number
        .checked_sub(provider.current_commitment_sequence_number)
        .ok_or(ProgramError::InvalidArgument)?;
    request.num_hashes = u32::try_from(num_hashes).map_err(|_| ProgramError::InvalidArgument)?;
    if provider.max_num_hashes != 0 && request.num_hashes > provider.max_num_hashes {
        return Err(EntropyError::LastRevealedTooOld.into());
    }

    request.commitment = hashv(&[&args.user_commitment, &provider.current_commitment]).to_bytes();
    request.requester_program_id = requester_program.key.to_bytes();
    request.request_slot = Clock::get()?.slot;
    request.use_blockhash = args.use_blockhash;
    request.callback_status = CALLBACK_NOT_NECESSARY;
    request.compute_unit_limit = provider.default_compute_unit_limit;
    request.payer = payer.key.to_bytes();
    request.discriminator = request_discriminator();

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

fn init_request_account_mut<'a, 'info>(
    program_id: &Pubkey,
    payer: &AccountInfo<'info>,
    request_account: &'a AccountInfo<'info>,
    system_program_account: &AccountInfo<'info>,
    space: usize,
) -> Result<RefMut<'a, Request>, ProgramError> {
    let rent = Rent::get()?;
    let required_lamports = rent.minimum_balance(space);
    if request_account.lamports() == 0 {
        let create_ix = system_instruction::create_account(
            payer.key,
            request_account.key,
            required_lamports,
            space as u64,
            program_id,
        );
        invoke(
            &create_ix,
            &[
                payer.clone(),
                request_account.clone(),
                system_program_account.clone(),
            ],
        )?;
    } else {
        let current_lamports = request_account.lamports();
        if current_lamports < required_lamports {
            let top_up = required_lamports
                .checked_sub(current_lamports)
                .ok_or(ProgramError::InvalidArgument)?;
            let transfer_ix = system_instruction::transfer(payer.key, request_account.key, top_up);
            invoke(
                &transfer_ix,
                &[
                    payer.clone(),
                    request_account.clone(),
                    system_program_account.clone(),
                ],
            )?;
        }

        let allocate_ix = system_instruction::allocate(request_account.key, space as u64);
        invoke(
            &allocate_ix,
            &[request_account.clone(), system_program_account.clone()],
        )?;

        let assign_ix = system_instruction::assign(request_account.key, program_id);
        invoke(
            &assign_ix,
            &[request_account.clone(), system_program_account.clone()],
        )?;
    }

    if request_account.owner != program_id || request_account.data_len() != space {
        return Err(EntropyError::InvalidAccount.into());
    }

    let data = request_account.data.borrow_mut();
    Ok(RefMut::map(data, |data| from_bytes_mut::<Request>(data)))
}
