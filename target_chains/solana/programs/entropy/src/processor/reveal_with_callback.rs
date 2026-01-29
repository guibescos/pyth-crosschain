use bytemuck::try_from_bytes;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    hash::{hash, hashv},
    program::{invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
    system_program,
    sysvar::{slot_hashes, slot_hashes::SlotHashes, Sysvar},
};

use crate::{
    accounts::{Provider, Request},
    constants::{
        CALLBACK_FAILED, CALLBACK_NOT_STARTED, ENTROPY_SIGNER_SEED, MAX_CALLBACK_ACCOUNTS,
    },
    discriminator::{provider_discriminator, request_discriminator},
    error::EntropyError,
    instruction::RevealArgs,
    pda::{entropy_signer_pda, provider_pda},
    pda_loader::load_account_mut,
};

pub fn process_reveal_with_callback(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let args = parse_reveal_args(data)?;

    let mut account_info_iter = accounts.iter();
    let request_account = next_account_info(&mut account_info_iter)?;
    let provider_account = next_account_info(&mut account_info_iter)?;
    let slot_hashes_account = next_account_info(&mut account_info_iter)?;
    let entropy_signer_account = next_account_info(&mut account_info_iter)?;
    let callback_program = next_account_info(&mut account_info_iter)?;
    let system_program_account = next_account_info(&mut account_info_iter)?;

    if !request_account.is_writable || !provider_account.is_writable {
        return Err(EntropyError::InvalidAccount.into());
    }

    if system_program_account.key != &system_program::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    if slot_hashes_account.key != &slot_hashes::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    let (expected_entropy_signer, _bump) = entropy_signer_pda(program_id);
    if entropy_signer_account.key != &expected_entropy_signer {
        return Err(EntropyError::InvalidPda.into());
    }

    let mut request = load_account_mut::<Request>(
        request_account,
        program_id,
        Request::LEN,
        request_discriminator(),
    )?;

    if request.callback_status != CALLBACK_NOT_STARTED
        && request.callback_status != CALLBACK_FAILED
    {
        return Err(EntropyError::InvalidRevealCall.into());
    }

    let request_provider = Pubkey::new_from_array(request.provider);
    if request_provider.to_bytes() != args.provider {
        return Err(EntropyError::InvalidAccount.into());
    }
    if request.sequence_number != args.sequence_number {
        return Err(EntropyError::InvalidAccount.into());
    }

    let (expected_provider, _provider_bump) = provider_pda(program_id, &request_provider);
    if provider_account.key != &expected_provider {
        return Err(EntropyError::InvalidPda.into());
    }

    let mut provider = load_account_mut::<Provider>(
        provider_account,
        program_id,
        Provider::LEN,
        provider_discriminator(),
    )?;

    let provider_commitment =
        hash_provider_commitment(args.provider_contribution, request.num_hashes)?;
    let user_commitment = hash(&args.user_contribution).to_bytes();
    let commitment = hashv(&[&user_commitment, &provider_commitment]).to_bytes();
    if commitment != request.commitment {
        return Err(EntropyError::IncorrectRevelation.into());
    }

    let blockhash = if request.use_blockhash == 1 {
        let slot_hashes = SlotHashes::from_account_info(slot_hashes_account)?;
        slot_hashes
            .iter()
            .find(|(slot, _)| *slot == request.request_slot)
            .map(|(_, hash)| hash.to_bytes())
            .ok_or(EntropyError::BlockhashUnavailable)?
    } else {
        [0u8; 32]
    };

    let random_number =
        hashv(&[&args.user_contribution, &args.provider_contribution, &blockhash]).to_bytes();

    if provider.current_commitment_sequence_number < request.sequence_number {
        provider.current_commitment_sequence_number = request.sequence_number;
        provider.current_commitment = args.provider_contribution;
    }

    let callback_program_id = Pubkey::new_from_array(request.callback_program_id);
    if callback_program_id != Pubkey::default() && callback_program.key != &callback_program_id {
        return Err(EntropyError::InvalidAccount.into());
    }

    let callback_accounts_len = request.callback_accounts_len as usize;
    if callback_accounts_len > MAX_CALLBACK_ACCOUNTS {
        return Err(EntropyError::InvalidAccount.into());
    }

    let remaining_accounts = account_info_iter.as_slice();
    if remaining_accounts.len() < callback_accounts_len {
        return Err(EntropyError::InvalidAccount.into());
    }

    let (callback_accounts, extra_accounts) = remaining_accounts.split_at(callback_accounts_len);
    validate_callback_accounts(&request, callback_accounts)?;

    let callback_ix_data_len = request.callback_ix_data_len;
    let callback_ix_data = request.callback_ix_data;
    let request_sequence_number = request.sequence_number;
    let request_provider_bytes = request.provider;
    let refund_pubkey = request.payer;

    if callback_program_id != Pubkey::default() {
        let callback_ix = build_callback_ix(
            callback_program.key,
            callback_accounts,
            callback_ix_data_len,
            &callback_ix_data,
            request_sequence_number,
            request_provider_bytes,
            random_number,
        )?;

        let bump_seed = [_bump];
        let signer_seeds: &[&[u8]] = &[ENTROPY_SIGNER_SEED, &bump_seed];
        let invoke_result =
            invoke_signed(&callback_ix, callback_accounts, &[signer_seeds]);

        if invoke_result.is_err() && request.callback_status == CALLBACK_NOT_STARTED {
            request.callback_status = CALLBACK_FAILED;
            return Ok(());
        }

        invoke_result?;
    }

    let refund_account = extra_accounts
        .get(0)
        .ok_or(EntropyError::InvalidAccount)?;
    let expected_refund = Pubkey::new_from_array(refund_pubkey);
    if refund_account.key != &expected_refund || !refund_account.is_writable {
        return Err(EntropyError::InvalidAccount.into());
    }

    drop(request);
    close_request_account(request_account, refund_account)?;

    Ok(())
}

fn parse_reveal_args(data: &[u8]) -> Result<&RevealArgs, ProgramError> {
    if data.len() != core::mem::size_of::<RevealArgs>() {
        return Err(ProgramError::InvalidInstructionData);
    }
    try_from_bytes::<RevealArgs>(data).map_err(|_| ProgramError::InvalidInstructionData)
}

fn hash_provider_commitment(
    mut provider_contribution: [u8; 32],
    num_hashes: u32,
) -> Result<[u8; 32], ProgramError> {
    for _ in 0..num_hashes {
        provider_contribution = hash(&provider_contribution).to_bytes();
    }
    Ok(provider_contribution)
}

fn validate_callback_accounts(
    request: &Request,
    callback_accounts: &[AccountInfo],
) -> ProgramResult {
    for (index, account_info) in callback_accounts.iter().enumerate() {
        let expected = request.callback_accounts[index];
        if account_info.key.to_bytes() != expected.pubkey {
            return Err(EntropyError::InvalidAccount.into());
        }
        if account_info.is_signer != (expected.is_signer == 1) {
            return Err(EntropyError::InvalidAccount.into());
        }
        if account_info.is_writable != (expected.is_writable == 1) {
            return Err(EntropyError::InvalidAccount.into());
        }
    }
    Ok(())
}

fn build_callback_ix(
    program_id: &Pubkey,
    callback_accounts: &[AccountInfo],
    callback_ix_data_len: u16,
    callback_ix_data: &[u8],
    sequence_number: u64,
    provider: [u8; 32],
    random_number: [u8; 32],
) -> Result<solana_program::instruction::Instruction, ProgramError> {
    let prefix_len = usize::from(callback_ix_data_len);
    if prefix_len > callback_ix_data.len() {
        return Err(ProgramError::InvalidInstructionData);
    }

    let mut data = Vec::with_capacity(prefix_len + 8 + 32 + 32);
    data.extend_from_slice(&callback_ix_data[..prefix_len]);
    data.extend_from_slice(&sequence_number.to_le_bytes());
    data.extend_from_slice(&provider);
    data.extend_from_slice(&random_number);

    let metas = callback_accounts
        .iter()
        .map(|info| solana_program::instruction::AccountMeta {
            pubkey: *info.key,
            is_signer: info.is_signer,
            is_writable: info.is_writable,
        })
        .collect();

    Ok(solana_program::instruction::Instruction {
        program_id: *program_id,
        accounts: metas,
        data,
    })
}

fn close_request_account(
    request_account: &AccountInfo,
    refund_account: &AccountInfo,
) -> ProgramResult {
    let lamports = request_account.lamports();
    let refund_lamports = refund_account
        .lamports()
        .checked_add(lamports)
        .ok_or(ProgramError::InvalidArgument)?;

    **request_account.try_borrow_mut_lamports()? = 0;
    **refund_account.try_borrow_mut_lamports()? = refund_lamports;

    let mut data = request_account.data.borrow_mut();
    data.fill(0);
    Ok(())
}
