use solana_program::pubkey::Pubkey;

use crate::constants::{
    CONFIG_SEED, ENTROPY_SIGNER_SEED, PROVIDER_SEED, PROVIDER_VAULT_SEED, PYTH_FEE_VAULT_SEED,
    REQUEST_SEED,
};

pub fn config_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CONFIG_SEED], program_id)
}

pub fn provider_pda(program_id: &Pubkey, provider_authority: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[PROVIDER_SEED, provider_authority.as_ref()], program_id)
}

pub fn provider_vault_pda(program_id: &Pubkey, provider_authority: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[PROVIDER_VAULT_SEED, provider_authority.as_ref()], program_id)
}

pub fn request_pda(
    program_id: &Pubkey,
    provider_authority: &Pubkey,
    sequence_number: u64,
) -> (Pubkey, u8) {
    let sequence_bytes = sequence_number.to_le_bytes();
    Pubkey::find_program_address(
        &[REQUEST_SEED, provider_authority.as_ref(), &sequence_bytes],
        program_id,
    )
}

pub fn pyth_fee_vault_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[PYTH_FEE_VAULT_SEED], program_id)
}

pub fn entropy_signer_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ENTROPY_SIGNER_SEED], program_id)
}
