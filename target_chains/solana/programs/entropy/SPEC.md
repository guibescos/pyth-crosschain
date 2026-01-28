# Entropy Solana Program Mapping Spec

This document maps the Ethereum Entropy contract to a Solana program design. It is intended to guide the
Pinocchio implementation in `target_chains/solana/programs/entropy` and the Mollusk tests.

Ethereum reference sources:
- `target_chains/ethereum/contracts/contracts/entropy/Entropy.sol`
- `target_chains/ethereum/contracts/contracts/entropy/EntropyState.sol`
- `target_chains/ethereum/contracts/contracts/entropy/EntropyGovernance.sol`
- `target_chains/ethereum/contracts/contracts/entropy/EntropyUpgradable.sol`
- `target_chains/ethereum/entropy_sdk/solidity/EntropyStructs.sol`
- `target_chains/ethereum/entropy_sdk/solidity/EntropyStructsV2.sol`
- `target_chains/ethereum/entropy_sdk/solidity/EntropyErrors.sol`
- `target_chains/ethereum/entropy_sdk/solidity/EntropyEvents.sol`
- `target_chains/ethereum/entropy_sdk/solidity/EntropyEventsV2.sol`
- `target_chains/ethereum/entropy_sdk/solidity/EntropyStatusConstants.sol`

## 1. High-level mapping

Ethereum Entropy is a provider-based commit/reveal RNG protocol with fees and optional callbacks. The
Solana program mirrors the same protocol with explicit accounts for provider state, requests, and fee
vaults.

Key differences driven by Solana:
- Storage is explicit via PDAs instead of EVM mappings/arrays.
- Fees are held in PDA-owned vault accounts and transferred via system instructions.
- Callbacks are CPIs to the requester program (if provided). The request stores the callback program id
  plus the full callback account metas and callback instruction data to replay at reveal. Callback programs must authenticate the
  caller via the `entropy_signer` PDA (not via `callback_program_id` alone).
- "Gas limit" becomes a compute-unit limit hint (still stored for compatibility and fee calculation).
- Blockhash use is implemented via Sysvar SlotHashes instead of EVM `blockhash`.

## 2. Program accounts and PDAs

### 2.1 Config (global state)
PDA: `seeds = ["config"]`

Fields (fixed-size; use zero-copy/POD layout, no Borsh):
- `admin: Pubkey`
- `pyth_fee_lamports: u64`
- `accrued_pyth_fees_lamports: u64`
- `default_provider: Pubkey`
- `proposed_admin: Pubkey` (zero pubkey if none)
- `seed: [u8; 32]` (for PRNG used by requestV2 convenience methods)
- `bump: u8`

Notes:
- This replaces `EntropyState.State.admin`, `pythFeeInWei`, `accruedPythFeesInWei`, `defaultProvider`,
  `proposedAdmin`, and `seed`.

### 2.2 Provider account
PDA: `seeds = ["provider", provider_authority_pubkey]`

The provider authority is the signer on register/update/withdraw.

Fields (use zero-copy/POD layout; fixed-size):
- `provider_authority: Pubkey` (redundant but explicit)
- `fee_lamports: u64`
- `accrued_fees_lamports: u64`
- `original_commitment: [u8; 32]`
- `original_commitment_sequence_number: u64`
- `commitment_metadata_len: u16`
- `commitment_metadata: [u8; COMMITMENT_METADATA_LEN]`
- `uri_len: u16`
- `uri: [u8; URI_LEN]`
- `end_sequence_number: u64`
- `sequence_number: u64` (next sequence number to assign)
- `current_commitment: [u8; 32]`
- `current_commitment_sequence_number: u64`
- `fee_manager: Pubkey` (zero pubkey if none)
- `max_num_hashes: u32`
- `default_compute_unit_limit: u32`
- `bump: u8`

Notes:
- Mirrors `EntropyStructsV2.ProviderInfo` and Ethereum registration semantics.
- `commitment_metadata` and `uri` are fixed-size, zero-padded buffers. Use `*_len` to indicate
  the valid prefix. Recommended constants: `COMMITMENT_METADATA_LEN = 64`, `URI_LEN = 256`.

### 2.3 Provider fee vault
PDA: `seeds = ["provider_vault", provider_authority_pubkey]`

System account holding lamports that back `provider.accrued_fees_lamports`.

### 2.4 Request account
PDA: `seeds = ["request", provider_authority_pubkey, sequence_number_le_bytes]`

Fields (fixed-size; use zero-copy/POD layout, no Borsh):
- `provider: Pubkey`
- `sequence_number: u64`
- `num_hashes: u32`
- `commitment: [u8; 32]` (sha256(user_commitment || provider_commitment))
- `request_slot: u64` (Solana slot at request time)
- `requester_program_id: Pubkey`
- `requester_signer: Pubkey` (PDA of requester program)
- `payer: Pubkey`
- `use_blockhash: bool`
- `callback_status: u8` (see Status Constants)
- `compute_unit_limit: u32` (stored as hint; fee calc uses this)
- `callback_program_id: Pubkey` (zero pubkey = no callback)
- `callback_accounts_len: u8`
- `callback_accounts: [CallbackMeta; MAX_CALLBACK_ACCOUNTS]`
- `callback_ix_data_len: u16`
- `callback_ix_data: [u8; CALLBACK_IX_DATA_LEN]`
- `bump: u8`

Notes:
- Replaces `EntropyStructsV2.Request` + callback status.
- The request account is created by the payer and closed on reveal; lamports returned to payer.
- `CallbackMeta` layout (fixed-size): `{ pubkey: Pubkey, is_signer: bool, is_writable: bool }`.
  The order of `callback_accounts` is the CPI account order.
- `callback_accounts` stores the full account metas supplied at request time. These are used to
  validate the accounts passed at reveal and to build the CPI.
- `callback_ix_data` stores the callback instruction data prefix. Reveal appends the Entropy
  callback payload `(sequence_number, provider, random_number)` after this prefix.
  Recommended constants: `MAX_CALLBACK_ACCOUNTS = 16`, `CALLBACK_IX_DATA_LEN = 256`.



### 2.5 Pyth fee vault
PDA: `seeds = ["pyth_fee_vault"]`

System account holding lamports that back `config.accrued_pyth_fees_lamports`.

### 2.6 Entropy signer (program-derived signer)
PDA: `seeds = ["entropy_signer"]`

Signer PDA used by the entropy program when invoking callback programs. The program should sign CPI
instructions with `invoke_signed` using `["entropy_signer", bump]`. Callback programs must verify that
the provided `entropy_signer` account matches `find_program_address(["entropy_signer"], entropy_program_id)`
and that it is a signer.

## 3. Status constants (mirror EntropyStatusConstants)

- `CALLBACK_NOT_NECESSARY = 0`
- `CALLBACK_NOT_STARTED = 1`
- `CALLBACK_IN_PROGRESS = 2`
- `CALLBACK_FAILED = 3`

## 4. Instructions

### 4.1 Initialize
Create config + pyth fee vault.

Accounts:
- `[signer]` payer
- `[writable]` config PDA
- `[writable]` pyth_fee_vault PDA
- `system_program`

Args:
- `admin: Pubkey`
- `pyth_fee_lamports: u64`
- `default_provider: Pubkey`

Checks:
- Admin and default provider are non-zero.

### 4.2 Register provider (create or rotate)
Mirrors `register` in EVM.

Accounts:
- `[signer]` provider_authority
- `[writable]` provider PDA (init if needed)
- `[writable]` provider_vault PDA (init if needed)
- `[writable]` config PDA
- `system_program`

Args:
- `fee_lamports: u64`
- `commitment: [u8; 32]`
- `commitment_metadata_len: u16`
- `commitment_metadata: [u8; COMMITMENT_METADATA_LEN]`
- `chain_length: u64`
- `uri_len: u16`
- `uri: [u8; URI_LEN]`

Behavior:
- Require `chain_length > 0`.
- Set provider fields like EVM:
  - `fee_lamports = fee_lamports`
  - `original_commitment = commitment`
  - `original_commitment_sequence_number = sequence_number`
  - `current_commitment = commitment`
  - `current_commitment_sequence_number = sequence_number`
  - `end_sequence_number = sequence_number + chain_length`
  - `commitment_metadata_len = ...`, `commitment_metadata = ...`
  - `uri_len = ...`, `uri = ...`
  - increment `sequence_number` by 1
- If provider already exists, update in-place (rotation).

### 4.3 Request (no callback)
Mirrors `request` in EVM.

Accounts:
- `[signer]` requester_signer (PDA of requester program)
- `[writable, signer]` payer system account
- `[readonly]` requester_program (invoker program id)
- `[writable]` request PDA (init)
- `[writable]` provider PDA
- `[writable]` provider_vault PDA
- `[writable]` config PDA
- `[writable]` pyth_fee_vault PDA
- `system_program`

Args:
- `provider: Pubkey`
- `user_commitment: [u8; 32]`
- `use_blockhash: bool`
- `compute_unit_limit: u32` (stored as 0 for no callback)

Behavior:
- Assign `sequence_number = provider.sequence_number` and increment it.
- Ensure `sequence_number < end_sequence_number` else `OutOfRandomness`.
- Compute `num_hashes = sequence_number - provider.current_commitment_sequence_number`.
- If `max_num_hashes != 0` and `num_hashes > max_num_hashes`, error `LastRevealedTooOld`.
- `commitment = sha256(user_commitment || provider.current_commitment)`.
- Verify `requester_signer` is the PDA derived by `requester_program` using
  `seeds = ["requester_signer", entropy_program_id]` and the provided bump, and
  require it to sign (via CPI `invoke_signed` from the requester program).
- Record `request_slot`, `requester_program_id`, `requester_signer`, `payer`, `use_blockhash`.
- `callback_status = CALLBACK_NOT_NECESSARY`, `callback_program_id = Pubkey::default()`.
- Fee: `required_fee = provider_fee + config.pyth_fee_lamports` where provider_fee scales
  by `compute_unit_limit` when `default_compute_unit_limit > 0` (see Fee Calculation).
- Transfer lamports from payer to provider_vault and pyth_fee_vault and bump accrued counters.

### 4.4 Request with callback (V2)
Mirrors `requestV2` and `requestWithCallback` in EVM.

Accounts:
- Same as Request + `callback_program` (readonly) + any callback accounts (readonly or writable).

Args:
- `provider: Pubkey`
- `user_randomness: [u8; 32]` (or none if using program PRNG)
- `compute_unit_limit: u32` (0 means provider default)
- `callback_accounts: Vec<CallbackMeta>`
- `callback_ix_data: Vec<u8>` (prefix bytes for the callback instruction)

Behavior:
- For requestV2 convenience, generate `user_randomness` via PRNG seeded from config.seed,
  current slot, recent blockhash, and requester_signer. Store back into config.seed.
- `user_commitment = sha256(user_randomness)`; `use_blockhash = false`.
- `callback_status = CALLBACK_NOT_STARTED`.
- Store `compute_unit_limit` (if 0, use provider default at reveal/fee calc).
- Store `callback_program_id`, `callback_accounts`, and `callback_ix_data`.
- Enforce `callback_accounts.len <= MAX_CALLBACK_ACCOUNTS` and
  `callback_ix_data.len <= CALLBACK_IX_DATA_LEN`.

### 4.5 Reveal (no callback)
Mirrors `reveal` in EVM.

Accounts:
- `[signer]` requester_signer
- `[writable]` payer (refund destination)
- `[writable]` request PDA
- `[writable]` provider PDA
- `slot_hashes` sysvar (readonly)
- `system_program` (for close)

Args:
- `provider: Pubkey`
- `sequence_number: u64`
- `user_contribution: [u8; 32]`
- `provider_contribution: [u8; 32]`

Behavior:
- Ensure request exists and matches provider/sequence.
- `callback_status` must be `CALLBACK_NOT_NECESSARY`.
- `requester_signer` must sign and match the PDA derived from
  `request.requester_program_id` with `seeds = ["requester_signer", entropy_program_id]`.
- `payer` must match `request.payer`.
- Verify commitment and compute random number (see Section 6).
- If `use_blockhash` true, load hash from `slot_hashes` using `request_slot`. If missing, error
  `BlockhashUnavailable`.
- Update provider current commitment if sequence_number is newer.
- Close request account (lamports to payer).

### 4.6 Reveal with callback
Mirrors `revealWithCallback` in EVM.

Accounts:
- `[writable]` request PDA
- `[writable]` provider PDA
- `slot_hashes` sysvar (readonly)
- `[readonly]` entropy_signer (PDA of entropy program)
- `[readonly]` callback_program (if callback required)
- `callback accounts` (remaining accounts; must match stored `callback_accounts`)
- `system_program` (for close)

Args:
- `provider: Pubkey`
- `sequence_number: u64`
- `user_contribution: [u8; 32]`
- `provider_contribution: [u8; 32]`

Behavior:
- `callback_status` must be `CALLBACK_NOT_STARTED` or `CALLBACK_FAILED`.
- Verify commitment and compute random number.
- `entropy_signer` must match `find_program_address(["entropy_signer"], entropy_program_id)` and
  be a signer (via `invoke_signed`).
- If `callback_program_id` is non-zero, verify the remaining accounts match the stored
  `callback_accounts` (pubkey + signer + writable). CPI into callback program with
  instruction data = `callback_ix_data || entropy_callback_payload`, where the payload
  encodes (sequence_number, provider, random_number). Recommended: define a Solana entropy
  callback interface for requesters.
- If CPI fails and status was NOT_STARTED, mark as CALLBACK_FAILED.
- If CPI succeeds, close request account.

### 4.7 Advance provider commitment
Mirrors `advanceProviderCommitment` in EVM.

Accounts:
- `[signer]` provider_authority
- `[writable]` provider PDA

Args:
- `advanced_sequence_number: u64`
- `provider_contribution: [u8; 32]`

Checks/behavior:
- Must be `advanced_sequence_number > current_commitment_sequence_number` and
  `< end_sequence_number`, else `UpdateTooOld` or `AssertionFailure`.
- Verify that hashing `provider_contribution` forward `num_hashes` times equals
  `current_commitment`.
- Update `current_commitment_sequence_number` and `current_commitment`.
- If `current_commitment_sequence_number >= sequence_number`, set
  `sequence_number = current_commitment_sequence_number + 1`.

### 4.8 Provider config updates
Mirror EVM setters. Each requires provider authority or fee manager as in EVM.

Instructions:
- `set_provider_fee(new_fee_lamports)`
- `set_provider_fee_as_fee_manager(provider_authority, new_fee_lamports)`
- `set_provider_uri(new_uri_len, new_uri)`
- `set_fee_manager(new_fee_manager)`
- `set_max_num_hashes(new_max)`
- `set_default_compute_unit_limit(new_limit)`

### 4.9 Withdraw provider fees

Instructions:
- `withdraw(amount, recipient)` (provider authority)
- `withdraw_as_fee_manager(provider_authority, amount, recipient)`

Accounts:
- `[signer]` provider authority or fee manager
- `[writable]` provider PDA
- `[writable]` provider_vault PDA
- `[writable]` recipient system account
- `system_program`

Checks:
- Sufficient accrued fees.

### 4.10 Governance/admin
Mirror `EntropyGovernance`.

Instructions:
- `propose_admin(new_admin)`
- `accept_admin()`
- `set_pyth_fee(new_fee_lamports)`
- `set_default_provider(new_default_provider)`
- `withdraw_fee(amount, recipient)`

Accounts:
- `[signer]` admin (or program upgrade authority if mapped)
- `[writable]` config PDA
- `[writable]` pyth_fee_vault PDA (for withdraw_fee)
- `[writable]` recipient system account
- `system_program`

Rules:
- Admin auth should mirror Ethereum's `_authoriseAdminAction`. In the absence of an on-chain
  owner, require `config.admin` to sign.

## 5. Fee calculation

Ethereum logic (see `getProviderFee`):
- Provider charges `fee` for `defaultGasLimit`.
- Requests with gas limit above default pay proportionally more.

Solana mapping:
- Replace gas limit with compute unit limit. `default_compute_unit_limit` behaves like EVM
  `defaultGasLimit`.
- `rounded_limit = round_up_to_10k(compute_unit_limit)` (unit = 10k CU).
- If `default_compute_unit_limit > 0` and `rounded_limit > default`,
  `additional = (rounded_limit - default) * fee / default`.

## 6. Hashing and randomness

- Use sha256: `sha256(user_commitment || provider_commitment)` and
  for `combine_random_values` = sha256(user || provider || blockhash).
- Provider commitment validation: hash `provider_contribution` forward `num_hashes`
  times with sha256; must equal `current_commitment`.
- `use_blockhash` uses Sysvar SlotHashes to retrieve the hash for `request_slot`.
  If not present, return `BlockhashUnavailable`.
- PRNG for requestV2 convenience should mix `config.seed`, current slot, recent blockhash,
  and requester pubkey.

## 7. Errors (mapping from EntropyErrors)

Suggested error enum (names match Solidity where possible):
- `AssertionFailure`
- `ProviderAlreadyRegistered` (unused; optional if register handles rotate)
- `NoSuchProvider`
- `NoSuchRequest`
- `OutOfRandomness`
- `InsufficientFee`
- `IncorrectRevelation`
- `InvalidUpgradeMagic` (unused on Solana; optional)
- `Unauthorized`
- `BlockhashUnavailable`
- `InvalidRevealCall`
- `LastRevealedTooOld`
- `UpdateTooOld`
- `InsufficientGas` (map to callback compute budget not sufficient)
- `MaxGasLimitExceeded` (map to compute unit limit too large)

## 8. Events/logs

Solana logs should mirror the logical events:
- Provider registered
- Request created
- Reveal completed
- Callback failed/succeeded
- Provider fee updated, URI updated, fee manager updated, max hashes updated, default limit updated
- Withdrawals

These can be program logs or a dedicated event account if needed by clients.

## 9. Pinocchio implementation notes

- Use zero-copy POD structs for state (fixed byte layout; no Borsh, no manual pack/unpack)
  to minimize compute units.
- Use `solana_program::hash::hash` for sha256.
- Enforce PDA seeds as described above; reject accounts with wrong PDA or owner.
- Validate signer/auth rules: provider authority for provider writes; admin for governance;
  requester for `reveal` (no callback).
- Store the full callback account metas and instruction data in the request account; validate
  the reveal remaining accounts against the stored metas before CPI.
- Close request accounts on success to reclaim rent.
- Keep instruction data small; define a compact instruction enum with fixed-size fields for
  common paths and reserve a variant only for truly variable-length inputs (e.g., callback
  account metas).

## 10. Data layout sizing (guidance)

Use fixed-size allocations with max lengths for provider metadata/URI and keep the constants
stable for deterministic sizing. If you need larger values, use a separate `ProviderMetadata`
PDA with fixed-size buffers plus `*_len` fields. For callbacks, cap `MAX_CALLBACK_ACCOUNTS`
and `CALLBACK_IX_DATA_LEN` to keep the request account deterministic.

Ensure the account sizes are deterministic for Mollusk tests.
