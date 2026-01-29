/// Fixed-size buffer length for provider commitment metadata.
pub const COMMITMENT_METADATA_LEN: usize = 64;
/// Fixed-size buffer length for provider URIs.
pub const URI_LEN: usize = 256;
/// Maximum number of callback accounts stored in a request.
pub const MAX_CALLBACK_ACCOUNTS: usize = 16;
/// Fixed-size buffer length for callback instruction data.
pub const CALLBACK_IX_DATA_LEN: usize = 256;

/// Seed for the config PDA.
pub const CONFIG_SEED: &[u8] = b"config";
/// Seed for the provider PDA.
pub const PROVIDER_SEED: &[u8] = b"provider";
/// Seed for the provider fee vault PDA.
pub const PROVIDER_VAULT_SEED: &[u8] = b"provider_vault";
/// Seed for the request PDA.
pub const REQUEST_SEED: &[u8] = b"request";
/// Seed for the Pyth fee vault PDA.
pub const PYTH_FEE_VAULT_SEED: &[u8] = b"pyth_fee_vault";
/// Seed for the entropy signer PDA.
pub const ENTROPY_SIGNER_SEED: &[u8] = b"entropy_signer";
/// Seed for the requester signer PDA (owned by requester program).
pub const REQUESTER_SIGNER_SEED: &[u8] = b"requester_signer";

/// Callback status constants (mirror EntropyStatusConstants).
pub const CALLBACK_NOT_NECESSARY: u8 = 0;
pub const CALLBACK_NOT_STARTED: u8 = 1;
pub const CALLBACK_IN_PROGRESS: u8 = 2;
