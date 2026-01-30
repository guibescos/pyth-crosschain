use bytemuck::from_bytes_mut;
#[allow(deprecated)]
use pinocchio::{
    cpi::invoke,
    sysvars::{clock::Clock, rent::Rent, Sysvar},
    AccountView,
    Address,
};
use pinocchio::error::ProgramError;
use pinocchio::account::RefMut;
use solana_sha256_hasher::hashv;

use crate::{
    accounts::{Config, Provider, Request},
    constants::CALLBACK_NOT_NECESSARY,
    discriminator::request_discriminator,
    error::EntropyError,
    instruction::RequestArgs,
    system_instruction,
};

#[allow(clippy::module_inception)]
mod request;
mod request_with_callback;
pub use request::process_request;
pub use request_with_callback::process_request_with_callback;

#[allow(clippy::too_many_arguments)]
fn request_helper<'a, 'info>(
    program_id: &Address,
    args: &RequestArgs,
    config: &Config,
    provider: &mut Provider,
    payer: &'a AccountView,
    requester_program: &'a AccountView,
    request_account: &'a AccountView,
    provider_vault: &'a AccountView,
    pyth_fee_vault: &'a AccountView,
    system_program_account: &'a AccountView,
) -> Result<u64, ProgramError> {
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
            system_instruction::transfer(payer.address(), provider_vault.address(), provider_fee);
        let instruction = transfer_ix.as_instruction();
        invoke(&instruction, &[payer, provider_vault, system_program_account])?;
    }
    if config.pyth_fee_lamports > 0 {
        let transfer_ix = system_instruction::transfer(
            payer.address(),
            pyth_fee_vault.address(),
            config.pyth_fee_lamports,
        );
        let instruction = transfer_ix.as_instruction();
        invoke(&instruction, &[payer, pyth_fee_vault, system_program_account])?;
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
    request.requester_program_id = requester_program.address().to_bytes();
    request.request_slot = Clock::get()?.slot;
    request.use_blockhash = args.use_blockhash;
    request.callback_status = CALLBACK_NOT_NECESSARY;
    request.compute_unit_limit = if args.compute_unit_limit > provider.default_compute_unit_limit {
        args.compute_unit_limit
    } else {
        provider.default_compute_unit_limit
    };
    request.payer = payer.address().to_bytes();
    request.discriminator = request_discriminator();

    Ok(sequence_number)
}

fn init_request_account_mut<'a, 'info>(
    program_id: &Address,
    payer: &AccountView,
    request_account: &'a AccountView,
    system_program_account: &AccountView,
    space: usize,
) -> Result<RefMut<'a, Request>, ProgramError> {
    let rent = Rent::get()?;
    let required_lamports = rent.minimum_balance(space);
    if request_account.lamports() == 0 {
        let create_ix = system_instruction::create_account(
            payer.address(),
            request_account.address(),
            required_lamports,
            space as u64,
            program_id,
        );
        let instruction = create_ix.as_instruction();
        invoke(&instruction, &[payer, request_account, system_program_account])?;
    } else {
        let current_lamports = request_account.lamports();
        if current_lamports < required_lamports {
            let top_up = required_lamports
                .checked_sub(current_lamports)
                .ok_or(ProgramError::InvalidArgument)?;
            let transfer_ix =
                system_instruction::transfer(payer.address(), request_account.address(), top_up);
            let instruction = transfer_ix.as_instruction();
            invoke(&instruction, &[payer, request_account, system_program_account])?;
        }

        let allocate_ix = system_instruction::allocate(request_account.address(), space as u64);
        let instruction = allocate_ix.as_instruction();
        invoke(&instruction, &[request_account, system_program_account])?;

        let assign_ix = system_instruction::assign(request_account.address(), program_id);
        let instruction = assign_ix.as_instruction();
        invoke(&instruction, &[request_account, system_program_account])?;
    }

    if !request_account.owned_by(program_id) || request_account.data_len() != space {
        return Err(EntropyError::InvalidAccount.into());
    }

    let data = request_account.try_borrow_mut()?;
    Ok(RefMut::map(data, |data| from_bytes_mut::<Request>(data)))
}
