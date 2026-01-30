#[allow(deprecated)]
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    program::invoke,
    system_instruction, system_program,
    sysvar::{rent::Rent, Sysvar},
};

use crate::error::EntropyError;

pub fn init_vault_pda<'a>(
    payer: &AccountInfo<'a>,
    vault: &AccountInfo<'a>,
    system_program_account: &AccountInfo<'a>,
) -> ProgramResult {
    if vault.owner != &system_program::ID || vault.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let rent = Rent::get()?;
    let required_vault_lamports = rent.minimum_balance(0);
    let current_vault_lamports = vault.lamports();
    if current_vault_lamports < required_vault_lamports {
        let transfer_ix = system_instruction::transfer(
            payer.key,
            vault.key,
            required_vault_lamports - current_vault_lamports,
        );
        invoke(
            &transfer_ix,
            &[payer.clone(), vault.clone(), system_program_account.clone()],
        )?;
    }

    Ok(())
}
