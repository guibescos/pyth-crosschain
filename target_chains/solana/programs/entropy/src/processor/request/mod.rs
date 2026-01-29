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
    discriminator::request_discriminator,
    error::EntropyError,
    instruction::RequestArgs,
    pda::{config_pda, provider_pda, provider_vault_pda, pyth_fee_vault_pda},
    pda_loader::{load_account, load_account_mut},
};

mod request;
pub use request::process_request;

fn request_helper<'a, 'info>(
    program_id: &Pubkey,
    args: &RequestArgs,
    config: &Config,
    provider: &mut Provider,
    payer: &'a AccountInfo<'info>,
    requester_program: &'a AccountInfo<'info>,
    request_account: &'a AccountInfo<'info>,
    provider_vault: &'a AccountInfo<'info>,
    pyth_fee_vault: &'a AccountInfo<'info>,
    system_program_account: &'a AccountInfo<'info>,
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
    request.compute_unit_limit = if args.compute_unit_limit > provider.default_compute_unit_limit {
        args.compute_unit_limit
    } else {
        provider.default_compute_unit_limit
    };
    request.payer = payer.key.to_bytes();
    request.discriminator = request_discriminator();

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
