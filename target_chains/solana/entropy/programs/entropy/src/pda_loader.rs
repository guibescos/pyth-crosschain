use bytemuck::from_bytes_mut;
#[allow(deprecated)]
use pinocchio::{
    cpi::{invoke, invoke_signed, Seed, Signer},
    sysvars::{rent::Rent, Sysvar},
    AccountView,
    Address,
};
use pinocchio::error::ProgramError;
use pinocchio::account::{Ref, RefMut};
use pinocchio_system as system_program;

use crate::{accounts::Account, error::EntropyError};
use crate::system_instruction;

pub fn load_account<'a, T: Account>(
    account: &'a AccountView,
    program_id: &Address,
) -> Result<Ref<'a, T>, ProgramError> {
    if !account.owned_by(program_id) || account.data_len() != T::LEN {
        return Err(EntropyError::InvalidAccount.into());
    }

    let data = account.try_borrow()?;
    let discriminator = data.get(0..8).ok_or(ProgramError::InvalidAccountData)?;
    if discriminator != T::discriminator() {
        return Err(EntropyError::InvalidAccount.into());
    }

    Ok(Ref::map(data, |data| bytemuck::from_bytes::<T>(data)))
}

pub fn load_account_mut<'a, 'info, T: Account>(
    account: &'a AccountView,
    program_id: &Address,
) -> Result<RefMut<'a, T>, ProgramError> {
    if !account.owned_by(program_id) || account.data_len() != T::LEN {
        return Err(EntropyError::InvalidAccount.into());
    }

    {
        let data = account.try_borrow()?;
        let discriminator = data.get(0..8).ok_or(ProgramError::InvalidAccountData)?;
        if discriminator != T::discriminator() {
            return Err(EntropyError::InvalidAccount.into());
        }
    }

    let data = account.try_borrow_mut()?;
    Ok(RefMut::map(data, |data| from_bytes_mut::<T>(data)))
}

pub fn init_pda_mut<'a, 'info, T: bytemuck::Pod>(
    program_id: &Address,
    payer: &AccountView,
    account: &'a AccountView,
    system_program_account: &AccountView,
    seeds: &[&[u8]],
    space: usize,
) -> Result<RefMut<'a, T>, ProgramError> {
    if !account.owned_by(&system_program::ID) || account.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let rent = Rent::get()?;
    let required_lamports = rent.minimum_balance(space);
    let current_lamports = account.lamports();

    if current_lamports == 0 {
        let create_ix = system_instruction::create_account(
            payer.address(),
            account.address(),
            required_lamports,
            space as u64,
            program_id,
        );
        let instruction = create_ix.as_instruction();
        let signer_seeds: Vec<Seed> = seeds.iter().copied().map(Seed::from).collect();
        let signer = Signer::from(signer_seeds.as_slice());
        invoke_signed(
            &instruction,
            &[payer, account, system_program_account],
            &[signer],
        )?;
    } else {
        if current_lamports < required_lamports {
            let transfer_ix = system_instruction::transfer(
                payer.address(),
                account.address(),
                required_lamports - current_lamports,
            );
            let instruction = transfer_ix.as_instruction();
            invoke(
                &instruction,
                &[payer, account, system_program_account],
            )?;
        }

        let allocate_ix = system_instruction::allocate(account.address(), space as u64);
        let instruction = allocate_ix.as_instruction();
        let signer_seeds: Vec<Seed> = seeds.iter().copied().map(Seed::from).collect();
        let signer = Signer::from(signer_seeds.as_slice());
        invoke_signed(&instruction, &[account, system_program_account], &[signer])?;

        let assign_ix = system_instruction::assign(account.address(), program_id);
        let instruction = assign_ix.as_instruction();
        let signer_seeds: Vec<Seed> = seeds.iter().copied().map(Seed::from).collect();
        let signer = Signer::from(signer_seeds.as_slice());
        invoke_signed(&instruction, &[account, system_program_account], &[signer])?;
    }

    let data = account.try_borrow_mut()?;
    Ok(RefMut::map(data, |data| from_bytes_mut::<T>(data)))
}
