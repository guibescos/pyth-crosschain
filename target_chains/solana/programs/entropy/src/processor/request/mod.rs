mod request;

use std::cell::RefMut;

use bytemuck::from_bytes_mut;
use solana_program::{
    account_info::AccountInfo,
    hash::hashv,
    program::invoke,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_instruction, system_program,
    sysvar::{clock::Clock, rent::Rent, Sysvar},
};

use crate::{
    accounts::{CallbackMeta, Config, Provider, Request},
    constants::{
        CALLBACK_IX_DATA_LEN, MAX_CALLBACK_ACCOUNTS, REQUESTER_SIGNER_SEED,
    },
    discriminator::{config_discriminator, provider_discriminator, request_discriminator},
    error::EntropyError,
    instruction::RequestArgs,
    pda::{config_pda, provider_pda, provider_vault_pda, pyth_fee_vault_pda},
    pda_loader::{load_pda, load_pda_mut},
};

pub use request::process_request;

pub struct RequestAccounts<'a, 'info> {
    pub requester_signer: &'a AccountInfo<'info>,
    pub payer: &'a AccountInfo<'info>,
    pub requester_program: &'a AccountInfo<'info>,
    pub request_account: &'a AccountInfo<'info>,
    pub provider_account: &'a AccountInfo<'info>,
    pub provider_vault: &'a AccountInfo<'info>,
    pub config_account: &'a AccountInfo<'info>,
    pub pyth_fee_vault: &'a AccountInfo<'info>,
    pub system_program_account: &'a AccountInfo<'info>,
}

pub struct RequestCallbackConfig<'a> {
    pub callback_program_id: Pubkey,
    pub callback_accounts: &'a [CallbackMeta],
    pub callback_ix_data: &'a [u8],
    pub callback_status: u8,
    pub compute_unit_limit: u32,
}

pub fn request_helper<'a, 'info>(
    program_id: &Pubkey,
    accounts: RequestAccounts<'a, 'info>,
    args: &RequestArgs,
    callback: RequestCallbackConfig<'a>,
) -> Result<u64, ProgramError> {
    if callback.callback_accounts.len() > MAX_CALLBACK_ACCOUNTS
        || callback.callback_ix_data.len() > CALLBACK_IX_DATA_LEN
    {
        return Err(ProgramError::InvalidInstructionData);
    }

    if !accounts.requester_signer.is_signer
        || !accounts.payer.is_signer
        || !accounts.request_account.is_signer
    {
        return Err(ProgramError::MissingRequiredSignature);
    }

    if !accounts.payer.is_writable
        || !accounts.request_account.is_writable
        || !accounts.provider_account.is_writable
        || !accounts.provider_vault.is_writable
        || !accounts.pyth_fee_vault.is_writable
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    if accounts.system_program_account.key != &system_program::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    let requester_signer_seed = [REQUESTER_SIGNER_SEED, program_id.as_ref()];
    let (expected_requester_signer, _bump) =
        Pubkey::find_program_address(&requester_signer_seed, accounts.requester_program.key);
    if accounts.requester_signer.key != &expected_requester_signer {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_config, _config_bump) = config_pda(program_id);
    if accounts.config_account.key != &expected_config {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_pyth_fee_vault, _pyth_fee_vault_bump) = pyth_fee_vault_pda(program_id);
    if accounts.pyth_fee_vault.key != &expected_pyth_fee_vault {
        return Err(EntropyError::InvalidPda.into());
    }
    if accounts.pyth_fee_vault.owner != &system_program::ID
        || accounts.pyth_fee_vault.data_len() != 0
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    if accounts.request_account.owner != &system_program::ID
        || accounts.request_account.data_len() != 0
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    let config = load_pda::<Config>(
        accounts.config_account,
        program_id,
        Config::LEN,
        config_discriminator(),
    )?;
    let mut provider = load_pda_mut::<Provider>(
        accounts.provider_account,
        program_id,
        Provider::LEN,
        provider_discriminator(),
    )?;
    let provider_authority = Pubkey::new_from_array(provider.provider_authority);
    let (expected_provider, _provider_bump) = provider_pda(program_id, &provider_authority);
    if accounts.provider_account.key != &expected_provider {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_provider_vault, _provider_vault_bump) =
        provider_vault_pda(program_id, &provider_authority);
    if accounts.provider_vault.key != &expected_provider_vault {
        return Err(EntropyError::InvalidPda.into());
    }
    if accounts.provider_vault.owner != &system_program::ID
        || accounts.provider_vault.data_len() != 0
    {
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
        let transfer_ix =
            system_instruction::transfer(accounts.payer.key, accounts.provider_vault.key, provider_fee);
        invoke(
            &transfer_ix,
            &[
                accounts.payer.clone(),
                accounts.provider_vault.clone(),
                accounts.system_program_account.clone(),
            ],
        )?;
    }
    if config.pyth_fee_lamports > 0 {
        let transfer_ix = system_instruction::transfer(
            accounts.payer.key,
            accounts.pyth_fee_vault.key,
            config.pyth_fee_lamports,
        );
        invoke(
            &transfer_ix,
            &[
                accounts.payer.clone(),
                accounts.pyth_fee_vault.clone(),
                accounts.system_program_account.clone(),
            ],
        )?;
    }

    let mut request = init_request_account_mut(
        program_id,
        accounts.payer,
        accounts.request_account,
        accounts.system_program_account,
        Request::LEN,
    )?;

    let num_hashes = sequence_number
        .checked_sub(provider.current_commitment_sequence_number)
        .ok_or(ProgramError::InvalidArgument)?;
    let num_hashes = u32::try_from(num_hashes).map_err(|_| ProgramError::InvalidArgument)?;
    if provider.max_num_hashes != 0 && num_hashes > provider.max_num_hashes {
        return Err(EntropyError::LastRevealedTooOld.into());
    }

    let mut callback_accounts =
        [CallbackMeta { pubkey: [0u8; 32], is_signer: 0, is_writable: 0 }; MAX_CALLBACK_ACCOUNTS];
    for (index, meta) in callback.callback_accounts.iter().enumerate() {
        callback_accounts[index] = *meta;
    }

    let mut callback_ix_data = [0u8; CALLBACK_IX_DATA_LEN];
    let callback_ix_data_len = callback.callback_ix_data.len();
    if callback_ix_data_len > 0 {
        callback_ix_data[..callback_ix_data_len].copy_from_slice(callback.callback_ix_data);
    }

    let compute_unit_limit = if callback.callback_status == crate::constants::CALLBACK_NOT_NECESSARY
    {
        provider.default_compute_unit_limit
    } else {
        callback.compute_unit_limit
    };

    *request = Request {
        discriminator: request_discriminator(),
        provider: provider.provider_authority,
        sequence_number,
        num_hashes,
        commitment: hashv(&[&args.user_commitment, &provider.current_commitment]).to_bytes(),
        _padding0: [0u8; 4],
        request_slot: Clock::get()?.slot,
        requester_program_id: accounts.requester_program.key.to_bytes(),
        requester_signer: accounts.requester_signer.key.to_bytes(),
        payer: accounts.payer.key.to_bytes(),
        use_blockhash: args.use_blockhash,
        callback_status: callback.callback_status,
        _padding1: [0u8; 2],
        compute_unit_limit,
        callback_program_id: callback.callback_program_id.to_bytes(),
        callback_accounts_len: callback.callback_accounts.len() as u8,
        _padding2: [0u8; 1],
        callback_accounts,
        callback_ix_data_len: callback_ix_data_len as u16,
        callback_ix_data,
        bump: 0,
        _padding3: [0u8; 3],
    };

    Ok(sequence_number)
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
