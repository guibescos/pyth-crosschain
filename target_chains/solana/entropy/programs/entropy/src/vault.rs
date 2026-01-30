#[allow(deprecated)]
use pinocchio::{
    cpi::invoke,
    sysvars::{rent::Rent, Sysvar},
    AccountView,
    ProgramResult,
};
use pinocchio_system as system_program;

use crate::{error::EntropyError, system_instruction};

pub fn init_vault_pda<'a>(
    payer: &AccountView,
    vault: &AccountView,
    system_program_account: &AccountView,
) -> ProgramResult {
    if !vault.owned_by(&system_program::ID) || vault.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let rent = Rent::get()?;
    let required_vault_lamports = rent.minimum_balance(0);
    let current_vault_lamports = vault.lamports();
    if current_vault_lamports < required_vault_lamports {
        let transfer_ix = system_instruction::transfer(
            payer.address(),
            vault.address(),
            required_vault_lamports - current_vault_lamports,
        );
        let instruction = transfer_ix.as_instruction();
        invoke(&instruction, &[payer, vault, system_program_account])?;
    }

    Ok(())
}
