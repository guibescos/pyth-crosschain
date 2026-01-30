#[allow(deprecated)]
use pinocchio::{
    cpi::{invoke_signed_with_bounds, Seed, Signer, MAX_CPI_ACCOUNTS},
    sysvars::{slot_hashes, slot_hashes::SlotHashes},
    AccountView,
    Address,
    ProgramResult,
};
use pinocchio::error::ProgramError;
use pinocchio_system as system_program;
use solana_sha256_hasher::{hash, hashv};

use crate::{
    accounts::{Provider, Request},
    constants::{CALLBACK_NOT_STARTED, ENTROPY_SIGNER_SEED, MAX_CALLBACK_ACCOUNTS},
    error::EntropyError,
    instruction::RevealArgs,
    load_account,
    pda::{entropy_signer_pda, provider_pda},
    pda_loader::load_account_mut,
    processor::{next_account_info, parse_args},
};

pub fn process_reveal_with_callback(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let args = parse_args::<RevealArgs>(data)?;

    let mut account_info_iter = accounts.iter();
    let request_account = next_account_info(&mut account_info_iter)?;
    let provider_account = next_account_info(&mut account_info_iter)?;
    let slot_hashes_account = next_account_info(&mut account_info_iter)?;
    let entropy_signer_account = next_account_info(&mut account_info_iter)?;
    let callback_program = next_account_info(&mut account_info_iter)?;
    let system_program_account = next_account_info(&mut account_info_iter)?;
    let payer_account = next_account_info(&mut account_info_iter)?;

    if !request_account.is_writable() || !provider_account.is_writable() {
        return Err(EntropyError::InvalidAccount.into());
    }

    if system_program_account.address() != &system_program::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    if slot_hashes_account.address() != &slot_hashes::SLOTHASHES_ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    let (expected_entropy_signer, _bump) = entropy_signer_pda(program_id);
    if entropy_signer_account.address() != &expected_entropy_signer {
        return Err(EntropyError::InvalidPda.into());
    }

    let request = load_account::<Request>(request_account, program_id)?;

    if request.callback_status != CALLBACK_NOT_STARTED {
        return Err(EntropyError::InvalidRevealCall.into());
    }

    let request_provider = Address::new_from_array(request.provider);

    let (expected_provider, _provider_bump) = provider_pda(program_id, &request_provider);
    if provider_account.address() != &expected_provider {
        return Err(EntropyError::InvalidPda.into());
    }

    let mut provider = load_account_mut::<Provider>(provider_account, program_id)?;

    let provider_commitment =
        hash_provider_commitment(args.provider_contribution, request.num_hashes)?;
    let user_commitment = hash(&args.user_contribution).to_bytes();
    let commitment = hashv(&[&user_commitment, &provider_commitment]).to_bytes();
    if commitment != request.commitment {
        return Err(EntropyError::IncorrectRevelation.into());
    }

    let blockhash = if request.use_blockhash == 1 {
        let slot_hashes = SlotHashes::from_account_view(slot_hashes_account)?;
        slot_hashes
            .entries()
            .iter()
            .find(|entry| entry.slot() == request.request_slot)
            .map(|entry| entry.hash)
            .ok_or(EntropyError::BlockhashUnavailable)?
    } else {
        [0u8; 32]
    };

    let random_number = hashv(&[
        &args.user_contribution,
        &args.provider_contribution,
        &blockhash,
    ])
    .to_bytes();

    if provider.current_commitment_sequence_number < request.sequence_number {
        provider.current_commitment_sequence_number = request.sequence_number;
        provider.current_commitment = args.provider_contribution;
    }

    let requester_program_id = Address::new_from_array(request.requester_program_id);
    if callback_program.address() != &requester_program_id {
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

    let (callback_accounts, _) = remaining_accounts.split_at(callback_accounts_len);
    validate_callback_accounts(&request, callback_accounts)?;

    let callback_ix_data_len = request.callback_ix_data_len;
    let callback_ix_data = request.callback_ix_data;
    let request_sequence_number = request.sequence_number;
    let request_provider_bytes = request.provider;
    let callback_compute_unit_limit = request.compute_unit_limit;

    if callback_compute_unit_limit != 0 && request.callback_status == CALLBACK_NOT_STARTED {
        let callback_ix = build_callback_ix(
            callback_program.address(),
            entropy_signer_account.address(),
            callback_accounts,
            callback_ix_data_len,
            &callback_ix_data,
            request_sequence_number,
            request_provider_bytes,
            random_number,
        )?;

        // let callback_compute_units_before = sol_remaining_compute_units();
        let bump_seed = [_bump];
        let signer_seeds = [
            Seed::from(ENTROPY_SIGNER_SEED),
            Seed::from(&bump_seed),
        ];
        let signer = Signer::from(&signer_seeds);
        let mut callback_account_infos =
            Vec::with_capacity(callback_accounts.len().saturating_add(1));
        callback_account_infos.push(entropy_signer_account);
        callback_account_infos.extend(callback_accounts);
        let instruction = callback_ix.as_instruction();
        invoke_signed_with_bounds::<{ MAX_CPI_ACCOUNTS }>(
            &instruction,
            &callback_account_infos,
            &[signer],
        )?;
        // let callback_compute_units_after: u64 = sol_remaining_compute_units();
        // let callback_compute_units_spent =
        //     callback_compute_units_before.saturating_sub(callback_compute_units_after);

        // if callback_compute_units_spent > u64::from(callback_compute_unit_limit) {
        //     return Err(EntropyError::InsufficientGas.into());
        // }
    }

    if payer_account.address() != &Address::new_from_array(request.payer)
        || !payer_account.is_writable()
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    drop(request);
    close_request_account(request_account, payer_account)?;

    Ok(())
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
    callback_accounts: &[AccountView],
) -> ProgramResult {
    for (index, account_info) in callback_accounts.iter().enumerate() {
        let expected = request.callback_accounts[index];
        if account_info.address().to_bytes() != expected.pubkey {
            return Err(EntropyError::InvalidAccount.into());
        }
        if account_info.is_signer() != (expected.is_signer == 1) {
            return Err(EntropyError::InvalidAccount.into());
        }
        if account_info.is_writable() != (expected.is_writable == 1) {
            return Err(EntropyError::InvalidAccount.into());
        }
    }
    Ok(())
}

struct CallbackInstruction<'a> {
    program_id: &'a Address,
    accounts: Vec<pinocchio::instruction::InstructionAccount<'a>>,
    data: Vec<u8>,
}

impl<'a> CallbackInstruction<'a> {
    fn as_instruction(&'a self) -> pinocchio::instruction::InstructionView<'a, 'a, 'a, 'a> {
        pinocchio::instruction::InstructionView {
            program_id: self.program_id,
            accounts: &self.accounts,
            data: &self.data,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_callback_ix<'a>(
    program_id: &'a Address,
    entropy_signer: &'a Address,
    callback_accounts: &'a [AccountView],
    callback_ix_data_len: u16,
    callback_ix_data: &'a [u8],
    sequence_number: u64,
    provider: [u8; 32],
    random_number: [u8; 32],
) -> Result<CallbackInstruction<'a>, ProgramError> {
    let prefix_len = usize::from(callback_ix_data_len);
    if prefix_len > callback_ix_data.len() {
        return Err(ProgramError::InvalidInstructionData);
    }

    let mut data = Vec::with_capacity(prefix_len + 8 + 32 + 32);
    data.extend_from_slice(&callback_ix_data[..prefix_len]);
    data.extend_from_slice(&sequence_number.to_le_bytes());
    data.extend_from_slice(&provider);
    data.extend_from_slice(&random_number);
    let mut metas = Vec::with_capacity(callback_accounts.len().saturating_add(1));
    metas.push(pinocchio::instruction::InstructionAccount::new(
        entropy_signer,
        false,
        true,
    ));
    metas.extend(callback_accounts.iter().map(|info| {
        pinocchio::instruction::InstructionAccount::new(
            info.address(),
            info.is_writable(),
            info.is_signer(),
        )
    }));

    Ok(CallbackInstruction {
        program_id,
        accounts: metas,
        data,
    })
}

fn close_request_account(
    request_account: &AccountView,
    refund_account: &AccountView,
) -> ProgramResult {
    let lamports = request_account.lamports();
    let refund_lamports = refund_account
        .lamports()
        .checked_add(lamports)
        .ok_or(ProgramError::InvalidArgument)?;

    request_account.set_lamports(0);
    refund_account.set_lamports(refund_lamports);
    Ok(())
}
