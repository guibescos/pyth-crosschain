use bytemuck::{from_bytes, from_bytes_mut, try_from_bytes, Pod, Zeroable};
use solana_program::{
    account_info::AccountInfo,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
    system_instruction,
    system_program,
    sysvar::{rent::Rent, Sysvar},
};
use std::cell::{Ref, RefMut};

use crate::error::EntropyError;

pub fn init_pda_account<'a, T: Pod + Zeroable>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    pda_account: &'a AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
    expected_pda: &Pubkey,
    seeds: &[&[u8]],
) -> Result<RefMut<'a, T>, ProgramError> {
    if pda_account.key != expected_pda {
        return Err(EntropyError::InvalidPda.into());
    }

    if system_program_account.key != &system_program::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    if pda_account.owner != &system_program::ID || pda_account.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let size = core::mem::size_of::<T>();
    let rent = Rent::get()?;
    let required_lamports = rent.minimum_balance(size);
    let current_lamports = pda_account.lamports();

    if current_lamports == 0 {
        let create_ix = system_instruction::create_account(
            payer.key,
            pda_account.key,
            required_lamports,
            size as u64,
            program_id,
        );
        invoke_signed(
            &create_ix,
            &[
                payer.clone(),
                pda_account.clone(),
                system_program_account.clone(),
            ],
            &[seeds],
        )?;
    } else {
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

        let allocate_ix = system_instruction::allocate(pda_account.key, size as u64);
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
    }

    if pda_account.data_len() != size {
        return Err(EntropyError::InvalidAccount.into());
    }

    let data = pda_account.data.borrow_mut();
    Ok(RefMut::map(data, |bytes| from_bytes_mut::<T>(&mut *bytes)))
}

pub fn load_pda_account<'a, T: Pod>(
    program_id: &Pubkey,
    pda_account: &'a AccountInfo<'a>,
    expected_pda: &Pubkey,
    expected_discriminator: [u8; 8],
) -> Result<Ref<'a, T>, ProgramError> {
    if pda_account.key != expected_pda {
        return Err(EntropyError::InvalidPda.into());
    }

    if pda_account.owner != program_id {
        return Err(EntropyError::InvalidAccount.into());
    }

    let size = core::mem::size_of::<T>();
    let data = pda_account.data.borrow();
    if data.len() != size {
        return Err(EntropyError::InvalidAccount.into());
    }

    if data.len() < expected_discriminator.len()
        || data[..expected_discriminator.len()] != expected_discriminator
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    if try_from_bytes::<T>(&data).is_err() {
        return Err(ProgramError::InvalidAccountData);
    }

    Ok(Ref::map(data, |bytes| from_bytes::<T>(bytes)))
}

pub fn load_pda_account_mut<'a, T: Pod>(
    program_id: &Pubkey,
    pda_account: &'a AccountInfo<'a>,
    expected_pda: &Pubkey,
    expected_discriminator: [u8; 8],
) -> Result<RefMut<'a, T>, ProgramError> {
    if pda_account.key != expected_pda {
        return Err(EntropyError::InvalidPda.into());
    }

    if pda_account.owner != program_id {
        return Err(EntropyError::InvalidAccount.into());
    }

    let size = core::mem::size_of::<T>();
    let data = pda_account.data.borrow_mut();
    if data.len() != size {
        return Err(EntropyError::InvalidAccount.into());
    }

    if data.len() < expected_discriminator.len()
        || data[..expected_discriminator.len()] != expected_discriminator
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    Ok(RefMut::map(data, |bytes| from_bytes_mut::<T>(&mut *bytes)))
}
