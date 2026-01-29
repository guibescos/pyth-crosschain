use crate::constants::{
    CALLBACK_IX_DATA_LEN, COMMITMENT_METADATA_LEN, MAX_CALLBACK_ACCOUNTS, URI_LEN,
};
use crate::discriminator::{config_discriminator, provider_discriminator, request_discriminator};
use bytemuck::{Pod, Zeroable};
use solana_program::program_error::ProgramError;

pub type PubkeyBytes = [u8; 32];

pub trait Account: Pod {
    const LEN: usize;
    fn discriminator() -> [u8; 8];
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct Config {
    pub discriminator: [u8; 8],
    pub admin: PubkeyBytes,
    pub pyth_fee_lamports: u64,
    pub default_provider: PubkeyBytes,
    pub proposed_admin: PubkeyBytes,
    pub seed: [u8; 32],
    pub bump: u8,
    pub _padding0: [u8; 7],
}

impl Config {
    pub const LEN: usize = core::mem::size_of::<Self>();
}

impl Account for Config {
    const LEN: usize = Self::LEN;

    fn discriminator() -> [u8; 8] {
        config_discriminator()
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct Provider {
    pub discriminator: [u8; 8],
    pub provider_authority: PubkeyBytes,
    pub fee_lamports: u64,
    pub original_commitment: [u8; 32],
    pub original_commitment_sequence_number: u64,
    pub commitment_metadata_len: u16,
    pub commitment_metadata: [u8; COMMITMENT_METADATA_LEN],
    pub uri_len: u16,
    pub uri: [u8; URI_LEN],
    pub _padding0: [u8; 4],
    pub end_sequence_number: u64,
    pub sequence_number: u64,
    pub current_commitment: [u8; 32],
    pub current_commitment_sequence_number: u64,
    pub fee_manager: PubkeyBytes,
    pub max_num_hashes: u32,
    pub default_compute_unit_limit: u32,
    pub bump: u8,
    pub _padding1: [u8; 7],
}

impl Provider {
    pub const LEN: usize = core::mem::size_of::<Self>();

    pub fn calculate_provider_fee(&self, compute_unit_limit: u32) -> Result<u64, ProgramError> {
        if self.default_compute_unit_limit > 0
            && compute_unit_limit > self.default_compute_unit_limit
        {
            let extra_limit = compute_unit_limit.saturating_sub(self.default_compute_unit_limit);
            let additional_fee = u64::from(extra_limit)
                .checked_mul(self.fee_lamports)
                .ok_or(ProgramError::InvalidArgument)?
                .checked_div(u64::from(self.default_compute_unit_limit))
                .ok_or(ProgramError::InvalidArgument)?;
            Ok(self.fee_lamports.saturating_add(additional_fee))
        } else {
            Ok(self.fee_lamports)
        }
    }
}

impl Account for Provider {
    const LEN: usize = Self::LEN;

    fn discriminator() -> [u8; 8] {
        provider_discriminator()
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct CallbackMeta {
    pub pubkey: PubkeyBytes,
    pub is_signer: u8,
    pub is_writable: u8,
}

impl CallbackMeta {
    pub const LEN: usize = core::mem::size_of::<Self>();
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct Request {
    pub discriminator: [u8; 8],
    pub provider: PubkeyBytes,
    pub sequence_number: u64,
    pub num_hashes: u32,
    pub commitment: [u8; 32],
    pub _padding0: [u8; 4],
    pub request_slot: u64,
    pub requester_program_id: PubkeyBytes,
    pub requester_signer: PubkeyBytes,
    pub payer: PubkeyBytes,
    pub use_blockhash: u8,
    pub callback_status: u8,
    pub _padding1: [u8; 2],
    pub compute_unit_limit: u32,
    pub callback_program_id: PubkeyBytes,
    pub callback_accounts_len: u8,
    pub _padding2: [u8; 1],
    pub callback_accounts: [CallbackMeta; MAX_CALLBACK_ACCOUNTS],
    pub callback_ix_data_len: u16,
    pub callback_ix_data: [u8; CALLBACK_IX_DATA_LEN],
    pub bump: u8,
    pub _padding3: [u8; 3],
}

impl Request {
    pub const LEN: usize = core::mem::size_of::<Self>();
}

impl Account for Request {
    const LEN: usize = Self::LEN;

    fn discriminator() -> [u8; 8] {
        request_discriminator()
    }
}
