use solana_program::{
    account_info::AccountInfo,
    program::invoke,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_instruction,
    system_program,
    sysvar::{rent::Rent, Sysvar},
};

use crate::error::EntropyError;

pub fn initialize_pda_account(
    program_id: &Pubkey,
    payer: &AccountInfo,
    pda_account: &AccountInfo,
    system_program_account: &AccountInfo,
    seeds: &[&[u8]],
    space: usize,
) -> Result<(), ProgramError> {
    if system_program_account.key != &system_program::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    if pda_account.owner != &system_program::ID || pda_account.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let rent = Rent::get()?;
    let required_lamports = rent.minimum_balance(space);
    let current_lamports = pda_account.lamports();

    if current_lamports == 0 {
        let create_ix = system_instruction::create_account(
            payer.key,
            pda_account.key,
            required_lamports,
            space as u64,
            program_id,
        );
        invoke_signed(
            &create_ix,
            &[payer.clone(), pda_account.clone(), system_program_account.clone()],
            &[seeds],
        )?;
        return Ok(());
    }

    if current_lamports < required_lamports {
        let transfer_ix = system_instruction::transfer(
            payer.key,
            pda_account.key,
            required_lamports - current_lamports,
        );
        invoke(
            &transfer_ix,
            &[
                payer.clone(),
                pda_account.clone(),
                system_program_account.clone(),
            ],
        )?;
    }

    let allocate_ix = system_instruction::allocate(pda_account.key, space as u64);
    invoke_signed(
        &allocate_ix,
        &[pda_account.clone(), system_program_account.clone()],
        &[seeds],
    )?;

    let assign_ix = system_instruction::assign(pda_account.key, program_id);
    invoke_signed(
        &assign_ix,
        &[pda_account.clone(), system_program_account.clone()],
        &[seeds],
    )?;

    Ok(())
}
