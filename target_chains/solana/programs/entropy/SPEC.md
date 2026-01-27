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
- Callbacks are CPIs to the requester program (if provided). The request may store an optional account
  hash to bind the callback accounts, but does not store the callback program id.
- "Gas limit" becomes a compute-unit limit hint (still stored for compatibility and fee calculation).
- Blockhash use is implemented via Sysvar SlotHashes instead of EVM `blockhash`.

## 2. Program accounts and PDAs

### 2.1 Config (global state)
PDA: `seeds = ["config"]`

Fields (Borsh, fixed-size):
- `admin: Pubkey`
- `pyth_fee_lamports: u64`
- `accrued_pyth_fees_lamports: u64`
- `default_provider: Pubkey`
- `proposed_admin: Pubkey` (zero pubkey if none)
- `seed: [u8; 32]` (for PRNG used by requestV2 convenience methods)
- `bump: u8`
- `version: u8`

Notes:
- This replaces `EntropyState.State.admin`, `pythFeeInWei`, `accruedPythFeesInWei`, `defaultProvider`,
  `proposedAdmin`, and `seed`.

### 2.2 Provider account
PDA: `seeds = ["provider", provider_authority_pubkey]`

The provider authority is the signer on register/update/withdraw.

Fields (Borsh; variable-size if storing metadata/uri inline):
- `provider_authority: Pubkey` (redundant but explicit)
- `fee_lamports: u64`
- `accrued_fees_lamports: u64`
- `original_commitment: [u8; 32]`
- `original_commitment_sequence_number: u64`
- `commitment_metadata: Vec<u8>` (optional)
- `uri: Vec<u8>` (optional)
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
- If a fixed-size account is desired, move `commitment_metadata` and `uri` into a separate
  `ProviderMetadata` PDA and store their hashes or pointers in the provider account.

### 2.3 Provider fee vault
PDA: `seeds = ["provider_vault", provider_authority_pubkey]`

System account holding lamports that back `provider.accrued_fees_lamports`.

### 2.4 Request account
PDA: `seeds = ["request", provider_authority_pubkey, sequence_number_le_bytes]`

Fields:
- `provider: Pubkey`
- `sequence_number: u64`
- `num_hashes: u32`
- `commitment: [u8; 32]` (keccak256(user_commitment || provider_commitment))
- `request_slot: u64` (Solana slot at request time)
- `requester: Pubkey` (requester program id)
- `use_blockhash: bool`
- `callback_status: u8` (see Status Constants)
- `compute_unit_limit: u32` (stored as hint; fee calc uses this)
- `callback_accounts_hash: [u8; 32]` (optional, zero if unused)
- `bump: u8`

Notes:
- Replaces `EntropyStructsV2.Request` + callback status.
- The request account is created by the requester and closed on reveal; lamports returned to requester.
- `callback_accounts_hash` is a keccak of the metas supplied at request time to bind
  callback accounts. If not used, enforce only that the requester PDA signs.

### 2.5 Pyth fee vault
PDA: `seeds = ["pyth_fee_vault"]`

System account holding lamports that back `config.accrued_pyth_fees_lamports`.

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
- `prefill_request_storage: bool` (no-op on Solana; kept for parity)

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
- `commitment_metadata: Vec<u8>`
- `chain_length: u64`
- `uri: Vec<u8>`

Behavior:
- Require `chain_length > 0`.
- Set provider fields like EVM:
  - `fee_lamports = fee_lamports`
  - `original_commitment = commitment`
  - `original_commitment_sequence_number = sequence_number`
  - `current_commitment = commitment`
  - `current_commitment_sequence_number = sequence_number`
  - `end_sequence_number = sequence_number + chain_length`
  - `commitment_metadata = ...`, `uri = ...`
  - increment `sequence_number` by 1
- If provider already exists, update in-place (rotation).

### 4.3 Request (no callback)
Mirrors `request` in EVM.

Accounts:
- `[signer]` payer
- `[writable]` payer system account
- `[signer]` requester PDA (seeds = ["entropy_requester", requester_program_id])
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
- `commitment = keccak(user_commitment || provider.current_commitment)`.
- Record `request_slot`, `requester`, `use_blockhash`.
- `callback_status = CALLBACK_NOT_NECESSARY`.
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
- `callback_accounts: Vec<CallbackMeta>` (only if storing hash; otherwise supplied in instruction and
  optionally validated)

Behavior:
- For requestV2 convenience, generate `user_randomness` via PRNG seeded from config.seed,
  current slot, recent blockhash, and requester. Store back into config.seed.
- `user_commitment = keccak(user_randomness)`; `use_blockhash = false`.
- `callback_status = CALLBACK_NOT_STARTED`.
- Store `compute_unit_limit` (if 0, use provider default at reveal/fee calc).
- Optionally store `callback_accounts_hash`. The callback program id is provided in the instruction
  accounts and is not stored in the request.

### 4.5 Reveal (no callback)
Mirrors `reveal` in EVM.

Accounts:
- `[signer]` requester PDA (seeds = ["entropy_requester", requester_program_id])
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
- The requester PDA must sign; the requester program id is used to derive the PDA.
- Verify commitment and compute random number (see Section 6).
- If `use_blockhash` true, load hash from `slot_hashes` using `request_slot`. If missing, error
  `BlockhashUnavailable`.
- Update provider current commitment if sequence_number is newer.
- Close request account (lamports to requester).

### 4.6 Reveal with callback
Mirrors `revealWithCallback` in EVM.

Accounts:
- `[writable]` request PDA
- `[writable]` provider PDA
- `slot_hashes` sysvar (readonly)
- `[readonly]` callback_program (if callback required)
- `callback accounts` (remaining accounts)
- `system_program` (for close)

Args:
- `provider: Pubkey`
- `sequence_number: u64`
- `user_contribution: [u8; 32]`
- `provider_contribution: [u8; 32]`

Behavior:
- `callback_status` must be `CALLBACK_NOT_STARTED` or `CALLBACK_FAILED`.
- Verify commitment and compute random number.
- CPI into callback program with (sequence_number, provider, random_number). Recommended: define a
  Solana entropy callback interface for requesters.
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
- `set_provider_uri(new_uri)`
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

- Use keccak256 to match Ethereum: `keccak(user_commitment || provider_commitment)` and
  for `combine_random_values` = keccak(user || provider || blockhash).
- Provider commitment validation: hash `provider_contribution` forward `num_hashes`
  times with keccak; must equal `current_commitment`.
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

- Use `solana_program::keccak::hash` to match EVM keccak.
- Enforce PDA seeds as described above; reject accounts with wrong PDA or owner.
- Validate signer/auth rules: provider authority for provider writes; admin for governance;
- requester PDA for `reveal` (no callback) and request creation; payer signs and funds any fees.
- Store `callback_accounts_hash` if you need to bind the accounts used in reveal; compute
  the hash from the full account metas array supplied at request time.
- Close request accounts on success to reclaim rent.
- Keep instruction data small; define a compact instruction enum with fixed-size fields for
  common paths and a variant for variable-length metadata.

## 10. Data layout sizing (guidance)

Because of variable-length fields, prefer either:
- Fixed-size allocations with max lengths (e.g., 128 bytes for metadata, 256 bytes for URI), or
- A separate `ProviderMetadata` PDA with serialized `Vec<u8>` fields.

Ensure the account sizes are deterministic for Mollusk tests.
