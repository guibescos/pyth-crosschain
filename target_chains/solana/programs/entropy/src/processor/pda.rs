use std::cell::{Ref, RefMut};

use bytemuck::from_bytes_mut;
use solana_program::{
    account_info::AccountInfo,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
    system_instruction,
    system_program,
    sysvar::{rent::Rent, Sysvar},
};

use crate::error::EntropyError;

pub fn load_pda<'a, T: bytemuck::Pod>(
    account: &'a AccountInfo<'a>,
    program_id: &Pubkey,
    expected_len: usize,
    expected_discriminator: [u8; 8],
) -> Result<Ref<'a, T>, ProgramError> {
    if account.owner != program_id || account.data_len() != expected_len {
        return Err(EntropyError::InvalidAccount.into());
    }

    let data = account.data.borrow();
    let discriminator = data
        .get(0..8)
        .ok_or(ProgramError::InvalidAccountData)?;
    if discriminator != expected_discriminator {
        return Err(EntropyError::InvalidAccount.into());
    }

    Ok(Ref::map(data, |data| bytemuck::from_bytes::<T>(data)))
}

pub fn load_pda_mut<'a, T: bytemuck::Pod>(
    account: &'a AccountInfo<'a>,
    program_id: &Pubkey,
    expected_len: usize,
    expected_discriminator: [u8; 8],
) -> Result<RefMut<'a, T>, ProgramError> {
    if account.owner != program_id || account.data_len() != expected_len {
        return Err(EntropyError::InvalidAccount.into());
    }

    {
        let data = account.data.borrow();
        let discriminator = data
            .get(0..8)
            .ok_or(ProgramError::InvalidAccountData)?;
        if discriminator != expected_discriminator {
            return Err(EntropyError::InvalidAccount.into());
        }
    }

    let data = account.data.borrow_mut();
    Ok(RefMut::map(data, |data| from_bytes_mut::<T>(data)))
}

pub fn init_pda_mut<'a, T: bytemuck::Pod>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    account: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    seeds: &[&[u8]],
    space: usize,
) -> Result<RefMut<'a, T>, ProgramError> {
    if account.owner != &system_program::ID || account.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let rent = Rent::get()?;
    let required_lamports = rent.minimum_balance(space);
    let current_lamports = account.lamports();

    if current_lamports == 0 {
        let create_ix = system_instruction::create_account(
            payer.key,
            account.key,
            required_lamports,
            space as u64,
            program_id,
        );
        invoke_signed(
            &create_ix,
            &[payer.clone(), account.clone(), system_program_account.clone()],
            &[seeds],
        )?;
    } else {
        if current_lamports < required_lamports {
            let transfer_ix = system_instruction::transfer(
                payer.key,
                account.key,
                required_lamports - current_lamports,
            );
            invoke(
                &transfer_ix,
                &[payer.clone(), account.clone(), system_program_account.clone()],
            )?;
        }

        let allocate_ix = system_instruction::allocate(account.key, space as u64);
        invoke_signed(
            &allocate_ix,
            &[account.clone(), system_program_account.clone()],
            &[seeds],
        )?;

        let assign_ix = system_instruction::assign(account.key, program_id);
        invoke_signed(
            &assign_ix,
            &[account.clone(), system_program_account.clone()],
            &[seeds],
        )?;
    }

    let data = account.data.borrow_mut();
    Ok(RefMut::map(data, |data| from_bytes_mut::<T>(data)))
}
