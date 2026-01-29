mod request;
mod request_v2;
mod request_with_callback;

use std::cell::RefMut;

use bytemuck::{from_bytes_mut, try_from_bytes};
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    hash::hashv,
    program::{invoke, set_return_data},
    program_error::ProgramError,
    pubkey::Pubkey,
    system_instruction, system_program,
    sysvar::{clock::Clock, rent::Rent, slot_hashes::SlotHashes, Sysvar},
};

use crate::{
    accounts::{CallbackMeta, Config, Provider, Request},
    constants::{
        CALLBACK_IX_DATA_LEN, CALLBACK_NOT_STARTED, MAX_CALLBACK_ACCOUNTS, REQUESTER_SIGNER_SEED,
    },
    discriminator::{config_discriminator, provider_discriminator, request_discriminator},
    error::EntropyError,
    pda::{config_pda, provider_pda, provider_vault_pda, pyth_fee_vault_pda},
};

use super::pda::{load_pda, load_pda_mut};

pub use request::process_request;
pub use request_v2::process_request_v2;
pub use request_with_callback::process_request_with_callback;

fn parse_pubkey(data: &[u8], offset: usize) -> Result<(Pubkey, usize), ProgramError> {
    let end = offset
        .checked_add(32)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let bytes: [u8; 32] = data
        .get(offset..end)
        .ok_or(ProgramError::InvalidInstructionData)?
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    Ok((Pubkey::new_from_array(bytes), end))
}

fn parse_bytes32(data: &[u8], offset: usize) -> Result<([u8; 32], usize), ProgramError> {
    let end = offset
        .checked_add(32)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let bytes: [u8; 32] = data
        .get(offset..end)
        .ok_or(ProgramError::InvalidInstructionData)?
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    Ok((bytes, end))
}

fn parse_u32(data: &[u8], offset: usize) -> Result<(u32, usize), ProgramError> {
    let end = offset
        .checked_add(4)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let bytes: [u8; 4] = data
        .get(offset..end)
        .ok_or(ProgramError::InvalidInstructionData)?
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    Ok((u32::from_le_bytes(bytes), end))
}

fn parse_callback_accounts(data: &[u8]) -> Result<(Vec<CallbackMeta>, usize), ProgramError> {
    let (len_u32, mut offset) = parse_u32(data, 0)?;
    let len = usize::try_from(len_u32).map_err(|_| ProgramError::InvalidInstructionData)?;
    if len > MAX_CALLBACK_ACCOUNTS {
        return Err(ProgramError::InvalidInstructionData);
    }
    let size = CallbackMeta::LEN
        .checked_mul(len)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let end = offset
        .checked_add(size)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let slice = data.get(offset..end).ok_or(ProgramError::InvalidInstructionData)?;

    let mut accounts = Vec::with_capacity(len);
    for chunk in slice.chunks_exact(CallbackMeta::LEN) {
        let meta = *try_from_bytes::<CallbackMeta>(chunk)
            .map_err(|_| ProgramError::InvalidInstructionData)?;
        if meta.is_signer > 1 || meta.is_writable > 1 {
            return Err(ProgramError::InvalidInstructionData);
        }
        accounts.push(meta);
    }
    offset = end;

    Ok((accounts, offset))
}

fn parse_callback_ix_data(data: &[u8]) -> Result<(Vec<u8>, usize), ProgramError> {
    let (len_u32, offset) = parse_u32(data, 0)?;
    let len = usize::try_from(len_u32).map_err(|_| ProgramError::InvalidInstructionData)?;
    if len > CALLBACK_IX_DATA_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let end = offset
        .checked_add(len)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let slice = data.get(offset..end).ok_or(ProgramError::InvalidInstructionData)?;
    Ok((slice.to_vec(), end))
}

fn process_request_with_callback_common(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    provider_authority: Pubkey,
    user_randomness: Option<[u8; 32]>,
    compute_unit_limit: u32,
    callback_accounts: Vec<CallbackMeta>,
    callback_ix_data: Vec<u8>,
) -> ProgramResult {
    if accounts.len() < 10 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let requester_signer = &accounts[0];
    let payer = &accounts[1];
    let requester_program = &accounts[2];
    let request_account = &accounts[3];
    let provider_account = &accounts[4];
    let provider_vault = &accounts[5];
    let config_account = &accounts[6];
    let pyth_fee_vault = &accounts[7];
    let callback_program = &accounts[8];
    let system_program_account = accounts
        .last()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    let callback_account_infos = &accounts[9..accounts.len() - 1];

    if callback_account_infos.len() != callback_accounts.len() {
        return Err(ProgramError::InvalidInstructionData);
    }

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

    let (pyth_fee_lamports, user_randomness) = match user_randomness {
        Some(value) => {
            let config = load_pda::<Config>(
                config_account,
                program_id,
                Config::LEN,
                config_discriminator(),
            )?;
            (config.pyth_fee_lamports, value)
        }
        None => {
            if !config_account.is_writable {
                return Err(EntropyError::InvalidAccount.into());
            }
            let mut config = load_pda_mut::<Config>(
                config_account,
                program_id,
                Config::LEN,
                config_discriminator(),
            )?;
            let slot = Clock::get()?.slot;
            let slot_bytes = slot.to_le_bytes();
            let slot_hashes = SlotHashes::get()?;
            let recent_blockhash = slot_hashes
                .iter()
                .next()
                .ok_or(ProgramError::InvalidInstructionData)?
                .1
                .to_bytes();
            let next_seed = hashv(&[
                &config.seed,
                &slot_bytes,
                &recent_blockhash,
                requester_signer.key.as_ref(),
            ])
            .to_bytes();
            config.seed = next_seed;
            (config.pyth_fee_lamports, next_seed)
        }
    };

    let mut provider = load_pda_mut::<Provider>(
        provider_account,
        program_id,
        Provider::LEN,
        provider_discriminator(),
    )?;

    let actual_provider_authority = Pubkey::new_from_array(provider.provider_authority);
    if actual_provider_authority != provider_authority {
        return Err(EntropyError::InvalidAccount.into());
    }
    let (expected_provider, _provider_bump) = provider_pda(program_id, &actual_provider_authority);
    if provider_account.key != &expected_provider {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_provider_vault, _provider_vault_bump) =
        provider_vault_pda(program_id, &actual_provider_authority);
    if provider_vault.key != &expected_provider_vault {
        return Err(EntropyError::InvalidPda.into());
    }
    if provider_vault.owner != &system_program::ID || provider_vault.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    for (meta, info) in callback_accounts.iter().zip(callback_account_infos.iter()) {
        if info.key.to_bytes() != meta.pubkey {
            return Err(EntropyError::InvalidAccount.into());
        }
        if meta.is_signer == 1 && !info.is_signer {
            return Err(EntropyError::InvalidAccount.into());
        }
        if meta.is_writable == 1 && !info.is_writable {
            return Err(EntropyError::InvalidAccount.into());
        }
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
    let fee_compute_unit_limit = if compute_unit_limit == 0 {
        provider.default_compute_unit_limit
    } else {
        compute_unit_limit
    };
    let provider_fee = provider.calculate_provider_fee(fee_compute_unit_limit)?;
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
    if pyth_fee_lamports > 0 {
        let transfer_ix =
            system_instruction::transfer(payer.key, pyth_fee_vault.key, pyth_fee_lamports);
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

    let user_commitment = hashv(&[&user_randomness]).to_bytes();

    request.provider = provider.provider_authority;
    request.sequence_number = sequence_number;

    let num_hashes = sequence_number
        .checked_sub(provider.current_commitment_sequence_number)
        .ok_or(ProgramError::InvalidArgument)?;
    request.num_hashes = u32::try_from(num_hashes).map_err(|_| ProgramError::InvalidArgument)?;
    if provider.max_num_hashes != 0 && request.num_hashes > provider.max_num_hashes {
        return Err(EntropyError::LastRevealedTooOld.into());
    }

    request.commitment = hashv(&[&user_commitment, &provider.current_commitment]).to_bytes();
    request.requester_program_id = requester_program.key.to_bytes();
    request.requester_signer = requester_signer.key.to_bytes();
    request.request_slot = Clock::get()?.slot;
    request.use_blockhash = 0;
    request.callback_status = CALLBACK_NOT_STARTED;
    request.compute_unit_limit = compute_unit_limit;
    request.callback_program_id = callback_program.key.to_bytes();
    request.callback_accounts_len =
        u8::try_from(callback_accounts.len()).map_err(|_| ProgramError::InvalidInstructionData)?;
    let empty_callback_meta = CallbackMeta {
        pubkey: [0u8; 32],
        is_signer: 0,
        is_writable: 0,
    };
    request.callback_accounts.fill(empty_callback_meta);
    for (idx, meta) in callback_accounts.iter().enumerate() {
        request.callback_accounts[idx] = *meta;
    }
    request.callback_ix_data_len = u16::try_from(callback_ix_data.len())
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    request.callback_ix_data.fill(0);
    if !callback_ix_data.is_empty() {
        request.callback_ix_data[..callback_ix_data.len()].copy_from_slice(&callback_ix_data);
    }
    request.payer = payer.key.to_bytes();
    request.discriminator = request_discriminator();

    set_return_data(&sequence_number.to_le_bytes());

    Ok(())
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
