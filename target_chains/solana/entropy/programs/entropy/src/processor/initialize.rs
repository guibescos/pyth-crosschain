#[allow(deprecated)]
use pinocchio::{AccountView, Address, ProgramResult};
use pinocchio::error::ProgramError;
use pinocchio_system as system_program;

use crate::{
    accounts::Config,
    constants::CONFIG_SEED,
    discriminator::config_discriminator,
    error::EntropyError,
    instruction::InitializeArgs,
    pda::{config_pda, pyth_fee_vault_pda},
    pda_loader::init_pda_mut,
    processor::{next_account_info, parse_args},
    vault::init_vault_pda,
};

pub fn process_initialize(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let args = parse_args::<InitializeArgs>(data)?;

    if args.admin == [0u8; 32] || args.default_provider == [0u8; 32] {
        return Err(ProgramError::InvalidArgument);
    }

    let mut account_info_iter = accounts.iter();
    let payer = next_account_info(&mut account_info_iter)?;
    let config_account = next_account_info(&mut account_info_iter)?;
    let pyth_fee_vault = next_account_info(&mut account_info_iter)?;
    let system_program_account = next_account_info(&mut account_info_iter)?;

    if !payer.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }

    if !payer.is_writable() || !config_account.is_writable() || !pyth_fee_vault.is_writable() {
        return Err(EntropyError::InvalidAccount.into());
    }

    if system_program_account.address() != &system_program::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    let (expected_config, config_bump) = config_pda(program_id);
    if config_account.address() != &expected_config {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_fee_vault, _fee_vault_bump) = pyth_fee_vault_pda(program_id);
    if pyth_fee_vault.address() != &expected_fee_vault {
        return Err(EntropyError::InvalidPda.into());
    }

    if !config_account.owned_by(&system_program::ID) || config_account.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let mut config = init_pda_mut::<Config>(
        program_id,
        payer,
        config_account,
        system_program_account,
        &[CONFIG_SEED, &[config_bump]],
        Config::LEN,
    )?;

    init_vault_pda(payer, pyth_fee_vault, system_program_account)?;

    *config = Config {
        discriminator: config_discriminator(),
        admin: args.admin,
        pyth_fee_lamports: args.pyth_fee_lamports,
        default_provider: args.default_provider,
        proposed_admin: [0u8; 32],
        seed: [0u8; 32],
        bump: config_bump,
        _padding0: [0u8; 7],
    };

    Ok(())
}
