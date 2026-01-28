use std::cell::RefMut;

use bytemuck::{from_bytes_mut, try_from_bytes};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    hash::hashv,
    program::invoke,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_instruction, system_program,
    sysvar::{clock::Clock, rent::Rent, slot_hashes::PodSlotHashes, Sysvar},
};

use crate::{
    accounts::{CallbackMeta, Config, Provider, Request},
    constants::{
        CALLBACK_NOT_NECESSARY, CALLBACK_NOT_STARTED, CALLBACK_IX_DATA_LEN,
        MAX_CALLBACK_ACCOUNTS, REQUESTER_SIGNER_SEED,
    },
    discriminator::{config_discriminator, provider_discriminator, request_discriminator},
    error::EntropyError,
    instruction::RequestArgs,
    pda::{config_pda, provider_pda, provider_vault_pda, pyth_fee_vault_pda},
};

use super::pda::load_pda_mut;

pub fn process_request(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
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
        || !config_account.is_writable
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

    let provider_authority = Pubkey::new_from_array(args.provider);
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

    if request_account.owner != &system_program::ID
        || request_account.data_len() != 0
        || request_account.lamports() != 0
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    let mut config = load_pda_mut::<Config>(
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

    if provider.provider_authority != provider_authority.to_bytes() {
        return Err(EntropyError::InvalidAccount.into());
    }

    let sequence_number = provider.sequence_number;
    if sequence_number >= provider.end_sequence_number {
        return Err(EntropyError::OutOfRandomness.into());
    }

    let num_hashes = sequence_number
        .checked_sub(provider.current_commitment_sequence_number)
        .ok_or(ProgramError::InvalidArgument)?;
    if provider.max_num_hashes != 0 && num_hashes > provider.max_num_hashes as u64 {
        return Err(EntropyError::LastRevealedTooOld.into());
    }
    let num_hashes = u32::try_from(num_hashes).map_err(|_| ProgramError::InvalidArgument)?;

    let commitment = hashv(&[&args.user_commitment, &provider.current_commitment]).to_bytes();

    let provider_fee = calculate_provider_fee(
        provider.fee_lamports,
        provider.default_compute_unit_limit,
        args.compute_unit_limit,
    )?;
    let _required_fee = provider_fee
        .checked_add(config.pyth_fee_lamports)
        .ok_or(ProgramError::InvalidArgument)?;

    let request_slot = Clock::get()?.slot;

    let mut request = init_request_account_mut(
        program_id,
        payer,
        request_account,
        system_program_account,
        Request::LEN,
    )?;

    *request = Request {
        discriminator: request_discriminator(),
        provider: args.provider,
        sequence_number,
        num_hashes,
        commitment,
        _padding0: [0u8; 4],
        request_slot,
        requester_program_id: requester_program.key.to_bytes(),
        requester_signer: requester_signer.key.to_bytes(),
        payer: payer.key.to_bytes(),
        use_blockhash: args.use_blockhash,
        callback_status: CALLBACK_NOT_NECESSARY,
        _padding1: [0u8; 2],
        compute_unit_limit: 0,
        callback_program_id: [0u8; 32],
        callback_accounts_len: 0,
        _padding2: [0u8; 1],
        callback_accounts: [CallbackMeta {
            pubkey: [0u8; 32],
            is_signer: 0,
            is_writable: 0,
        }; MAX_CALLBACK_ACCOUNTS],
        callback_ix_data_len: 0,
        callback_ix_data: [0u8; CALLBACK_IX_DATA_LEN],
        bump: 0,
        _padding3: [0u8; 3],
    };

    drop(request);

    if provider_fee > 0 {
        let transfer_ix = system_instruction::transfer(payer.key, provider_vault.key, provider_fee);
        invoke(
            &transfer_ix,
            &[payer.clone(), provider_vault.clone(), system_program_account.clone()],
        )?;
    }

    if config.pyth_fee_lamports > 0 {
        let transfer_ix = system_instruction::transfer(
            payer.key,
            pyth_fee_vault.key,
            config.pyth_fee_lamports,
        );
        invoke(
            &transfer_ix,
            &[
                payer.clone(),
                pyth_fee_vault.clone(),
                system_program_account.clone(),
            ],
        )?;
    }

    provider.accrued_fees_lamports = provider
        .accrued_fees_lamports
        .checked_add(provider_fee)
        .ok_or(ProgramError::InvalidArgument)?;
    config.accrued_pyth_fees_lamports = config
        .accrued_pyth_fees_lamports
        .checked_add(config.pyth_fee_lamports)
        .ok_or(ProgramError::InvalidArgument)?;

    provider.sequence_number = provider
        .sequence_number
        .checked_add(1)
        .ok_or(ProgramError::InvalidArgument)?;

    Ok(())
}

pub fn process_request_with_callback(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let args = parse_request_with_callback_args(data)?;

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
    let callback_program = next_account_info(&mut account_info_iter)?;
    let callback_account_infos = account_info_iter.as_slice();

    if !requester_signer.is_signer || !payer.is_signer || !request_account.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    if !payer.is_writable
        || !request_account.is_writable
        || !provider_account.is_writable
        || !provider_vault.is_writable
        || !config_account.is_writable
        || !pyth_fee_vault.is_writable
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    if system_program_account.key != &system_program::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    if callback_account_infos.len() != args.callback_accounts.len() {
        return Err(EntropyError::InvalidAccount.into());
    }

    let requester_signer_seed = [REQUESTER_SIGNER_SEED, program_id.as_ref()];
    let (expected_requester_signer, _bump) =
        Pubkey::find_program_address(&requester_signer_seed, requester_program.key);
    if requester_signer.key != &expected_requester_signer {
        return Err(EntropyError::InvalidPda.into());
    }

    let provider_authority = Pubkey::new_from_array(args.provider);
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

    if request_account.owner != &system_program::ID
        || request_account.data_len() != 0
        || request_account.lamports() != 0
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    if args.callback_accounts.len() > MAX_CALLBACK_ACCOUNTS {
        return Err(ProgramError::InvalidInstructionData);
    }
    if args.callback_ix_data.len() > CALLBACK_IX_DATA_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }

    let mut config = load_pda_mut::<Config>(
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

    if provider.provider_authority != provider_authority.to_bytes() {
        return Err(EntropyError::InvalidAccount.into());
    }

    let sequence_number = provider.sequence_number;
    if sequence_number >= provider.end_sequence_number {
        return Err(EntropyError::OutOfRandomness.into());
    }

    let num_hashes = sequence_number
        .checked_sub(provider.current_commitment_sequence_number)
        .ok_or(ProgramError::InvalidArgument)?;
    if provider.max_num_hashes != 0 && num_hashes > provider.max_num_hashes as u64 {
        return Err(EntropyError::LastRevealedTooOld.into());
    }
    let num_hashes = u32::try_from(num_hashes).map_err(|_| ProgramError::InvalidArgument)?;

    let user_randomness = match args.user_randomness {
        Some(randomness) => randomness,
        None => generate_user_randomness(&mut config, requester_signer.key)?,
    };
    let user_commitment = hashv(&[&user_randomness]).to_bytes();
    let commitment = hashv(&[&user_commitment, &provider.current_commitment]).to_bytes();

    let compute_unit_limit_for_fee = if args.compute_unit_limit == 0 {
        provider.default_compute_unit_limit
    } else {
        args.compute_unit_limit
    };
    let provider_fee = calculate_provider_fee(
        provider.fee_lamports,
        provider.default_compute_unit_limit,
        compute_unit_limit_for_fee,
    )?;
    let _required_fee = provider_fee
        .checked_add(config.pyth_fee_lamports)
        .ok_or(ProgramError::InvalidArgument)?;

    let request_slot = Clock::get()?.slot;

    let mut request = init_request_account_mut(
        program_id,
        payer,
        request_account,
        system_program_account,
        Request::LEN,
    )?;

    let mut callback_accounts = [CallbackMeta {
        pubkey: [0u8; 32],
        is_signer: 0,
        is_writable: 0,
    }; MAX_CALLBACK_ACCOUNTS];
    for (idx, meta) in args.callback_accounts.iter().enumerate() {
        callback_accounts[idx] = *meta;
    }

    let mut callback_ix_data = [0u8; CALLBACK_IX_DATA_LEN];
    callback_ix_data[..args.callback_ix_data.len()].copy_from_slice(&args.callback_ix_data);

    *request = Request {
        discriminator: request_discriminator(),
        provider: args.provider,
        sequence_number,
        num_hashes,
        commitment,
        _padding0: [0u8; 4],
        request_slot,
        requester_program_id: requester_program.key.to_bytes(),
        requester_signer: requester_signer.key.to_bytes(),
        payer: payer.key.to_bytes(),
        use_blockhash: 0,
        callback_status: CALLBACK_NOT_STARTED,
        _padding1: [0u8; 2],
        compute_unit_limit: args.compute_unit_limit,
        callback_program_id: callback_program.key.to_bytes(),
        callback_accounts_len: args
            .callback_accounts
            .len()
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
        _padding2: [0u8; 1],
        callback_accounts,
        callback_ix_data_len: args
            .callback_ix_data
            .len()
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
        callback_ix_data,
        bump: 0,
        _padding3: [0u8; 3],
    };

    drop(request);

    if provider_fee > 0 {
        let transfer_ix = system_instruction::transfer(payer.key, provider_vault.key, provider_fee);
        invoke(
            &transfer_ix,
            &[payer.clone(), provider_vault.clone(), system_program_account.clone()],
        )?;
    }

    if config.pyth_fee_lamports > 0 {
        let transfer_ix = system_instruction::transfer(
            payer.key,
            pyth_fee_vault.key,
            config.pyth_fee_lamports,
        );
        invoke(
            &transfer_ix,
            &[
                payer.clone(),
                pyth_fee_vault.clone(),
                system_program_account.clone(),
            ],
        )?;
    }

    provider.accrued_fees_lamports = provider
        .accrued_fees_lamports
        .checked_add(provider_fee)
        .ok_or(ProgramError::InvalidArgument)?;
    config.accrued_pyth_fees_lamports = config
        .accrued_pyth_fees_lamports
        .checked_add(config.pyth_fee_lamports)
        .ok_or(ProgramError::InvalidArgument)?;

    provider.sequence_number = provider
        .sequence_number
        .checked_add(1)
        .ok_or(ProgramError::InvalidArgument)?;

    Ok(())
}

fn parse_request_args(data: &[u8]) -> Result<&RequestArgs, ProgramError> {
    if data.len() != core::mem::size_of::<RequestArgs>() {
        return Err(ProgramError::InvalidInstructionData);
    }

    try_from_bytes::<RequestArgs>(data).map_err(|_| ProgramError::InvalidInstructionData)
}

struct RequestWithCallbackArgs {
    provider: [u8; 32],
    user_randomness: Option<[u8; 32]>,
    compute_unit_limit: u32,
    callback_accounts: Vec<CallbackMeta>,
    callback_ix_data: Vec<u8>,
}

fn parse_request_with_callback_args(data: &[u8]) -> Result<RequestWithCallbackArgs, ProgramError> {
    parse_request_with_callback_args_inner(data, true)
        .or_else(|_| parse_request_with_callback_args_inner(data, false))
}

fn parse_request_with_callback_args_inner(
    data: &[u8],
    has_user_randomness: bool,
) -> Result<RequestWithCallbackArgs, ProgramError> {
    let mut offset = 0usize;

    if data.len() < offset + 32 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut provider = [0u8; 32];
    provider.copy_from_slice(&data[offset..offset + 32]);
    offset += 32;

    let user_randomness = if has_user_randomness {
        if data.len() < offset + 32 {
            return Err(ProgramError::InvalidInstructionData);
        }
        let mut randomness = [0u8; 32];
        randomness.copy_from_slice(&data[offset..offset + 32]);
        offset += 32;
        Some(randomness)
    } else {
        None
    };

    if data.len() < offset + 4 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let compute_unit_limit = u32::from_le_bytes(
        data[offset..offset + 4]
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );
    offset += 4;

    let callback_accounts_len = read_u32_len(data, &mut offset)?;
    let callback_accounts_len = usize::try_from(callback_accounts_len)
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    let required_meta_bytes = callback_accounts_len
        .checked_mul(CallbackMeta::LEN)
        .ok_or(ProgramError::InvalidInstructionData)?;
    if data.len() < offset + required_meta_bytes {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut callback_accounts = Vec::with_capacity(callback_accounts_len);
    for _ in 0..callback_accounts_len {
        let mut pubkey = [0u8; 32];
        pubkey.copy_from_slice(&data[offset..offset + 32]);
        offset += 32;
        let is_signer = data[offset];
        offset += 1;
        let is_writable = data[offset];
        offset += 1;
        if is_signer > 1 || is_writable > 1 {
            return Err(ProgramError::InvalidInstructionData);
        }
        callback_accounts.push(CallbackMeta {
            pubkey,
            is_signer,
            is_writable,
        });
    }

    let callback_ix_data_len = read_u32_len(data, &mut offset)?;
    let callback_ix_data_len = usize::try_from(callback_ix_data_len)
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    if data.len() < offset + callback_ix_data_len {
        return Err(ProgramError::InvalidInstructionData);
    }
    let callback_ix_data = data[offset..offset + callback_ix_data_len].to_vec();
    offset += callback_ix_data_len;

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

fn read_u32_len(data: &[u8], offset: &mut usize) -> Result<u32, ProgramError> {
    if data.len() < *offset + 4 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let len = u32::from_le_bytes(
        data[*offset..*offset + 4]
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );
    *offset += 4;
    Ok(len)
}

fn generate_user_randomness(
    config: &mut Config,
    requester_signer: &Pubkey,
) -> Result<[u8; 32], ProgramError> {
    let clock = Clock::get()?;
    let slot_hashes = PodSlotHashes::fetch()?;
    let recent_hash = slot_hashes
        .as_slice()?
        .first()
        .ok_or(ProgramError::InvalidArgument)?
        .hash;
    let seed = hashv(&[
        &config.seed,
        &clock.slot.to_le_bytes(),
        recent_hash.as_ref(),
        requester_signer.as_ref(),
    ])
    .to_bytes();
    config.seed = seed;
    Ok(seed)
}

fn round_up_to_10k(limit: u32) -> u64 {
    let limit = limit as u64;
    if limit == 0 {
        return 0;
    }
    let remainder = limit % 10_000;
    if remainder == 0 {
        limit
    } else {
        limit + (10_000 - remainder)
    }
}

fn calculate_provider_fee(
    base_fee: u64,
    default_compute_unit_limit: u32,
    compute_unit_limit: u32,
) -> Result<u64, ProgramError> {
    if default_compute_unit_limit == 0 {
        return Ok(base_fee);
    }

    let rounded_limit = round_up_to_10k(compute_unit_limit);
    let default_limit = default_compute_unit_limit as u64;
    if rounded_limit <= default_limit {
        return Ok(base_fee);
    }

    let extra = rounded_limit
        .checked_sub(default_limit)
        .ok_or(ProgramError::InvalidArgument)?
        .checked_mul(base_fee)
        .ok_or(ProgramError::InvalidArgument)?
        .checked_div(default_limit)
        .ok_or(ProgramError::InvalidArgument)?;

    base_fee
        .checked_add(extra)
        .ok_or(ProgramError::InvalidArgument)
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

    if request_account.owner != program_id || request_account.data_len() != space {
        return Err(EntropyError::InvalidAccount.into());
    }

    let data = request_account.data.borrow_mut();
    Ok(RefMut::map(data, |data| from_bytes_mut::<Request>(data)))
}
