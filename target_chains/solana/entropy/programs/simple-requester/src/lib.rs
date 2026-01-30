use {
    bytemuck::{bytes_of, try_from_bytes, Pod, Zeroable},
    entropy::{
        constants::{ENTROPY_SIGNER_SEED, REQUESTER_SIGNER_SEED},
        instruction::{EntropyInstruction, RequestArgs},
    },
    solana_program::{
        account_info::{next_account_info, AccountInfo},
        entrypoint::ProgramResult,
        instruction::{AccountMeta, Instruction},
        msg,
        program::invoke_signed,
        program_error::ProgramError,
        pubkey::Pubkey,
    },
};

pub const REQUEST_ACTION: u8 = 0;
pub const REQUEST_WITH_CALLBACK_ACTION: u8 = 1;
pub const CALLBACK_ACTION: u8 = 0xCB;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CallbackState {
    pub sequence_number: u64,
    pub provider: [u8; 32],
    pub random_number: [u8; 32],
    pub called: u8,
    pub _padding: [u8; 7],
}

pub const CALLBACK_STATE_LEN: usize = core::mem::size_of::<CallbackState>();

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    if data.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }

    match data[0] {
        REQUEST_ACTION => process_request(program_id, accounts, &data[1..]),
        REQUEST_WITH_CALLBACK_ACTION => {
            process_request_with_callback(program_id, accounts, &data[1..])
        }
        CALLBACK_ACTION => process_callback(program_id, accounts, &data[1..]),
        _ => {
            if data.len() == core::mem::size_of::<RequestArgs>() {
                process_request(program_id, accounts, data)
            } else {
                Err(ProgramError::InvalidInstructionData)
            }
        }
    }
}

fn process_request(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let args = try_from_bytes::<RequestArgs>(data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    let mut account_info_iter = accounts.iter();
    let requester_signer = next_account_info(&mut account_info_iter)?;
    let payer = next_account_info(&mut account_info_iter)?;
    let requester_program = next_account_info(&mut account_info_iter)?;
    let request_account = next_account_info(&mut account_info_iter)?;
    let provider_account = next_account_info(&mut account_info_iter)?;
    let provider_vault = next_account_info(&mut account_info_iter)?;
    let config_account = next_account_info(&mut account_info_iter)?;
    let pyth_fee_vault = next_account_info(&mut account_info_iter)?;
    let system_program_account = next_account_info(&mut account_info_iter)?;
    let entropy_program = next_account_info(&mut account_info_iter)?;

    if requester_program.key != program_id {
        return Err(ProgramError::InvalidArgument);
    }

    let mut entropy_data = Vec::with_capacity(8 + core::mem::size_of::<RequestArgs>());
    entropy_data.extend_from_slice(&EntropyInstruction::Request.discriminator());
    entropy_data.extend_from_slice(bytes_of(args));

    let entropy_ix = Instruction {
        program_id: *entropy_program.key,
        data: entropy_data,
        accounts: vec![
            AccountMeta::new_readonly(*requester_signer.key, true),
            AccountMeta::new(*payer.key, true),
            AccountMeta::new_readonly(*requester_program.key, false),
            AccountMeta::new(*request_account.key, true),
            AccountMeta::new(*provider_account.key, false),
            AccountMeta::new(*provider_vault.key, false),
            AccountMeta::new_readonly(*config_account.key, false),
            AccountMeta::new(*pyth_fee_vault.key, false),
            AccountMeta::new_readonly(*system_program_account.key, false),
        ],
    };

    let (expected_signer, bump) = Pubkey::find_program_address(
        &[REQUESTER_SIGNER_SEED, entropy_program.key.as_ref()],
        program_id,
    );
    if requester_signer.key != &expected_signer {
        return Err(ProgramError::InvalidSeeds);
    }

    let signer_seeds: &[&[u8]] = &[REQUESTER_SIGNER_SEED, entropy_program.key.as_ref(), &[bump]];
    invoke_signed(
        &entropy_ix,
        &[
            requester_signer.clone(),
            payer.clone(),
            requester_program.clone(),
            request_account.clone(),
            provider_account.clone(),
            provider_vault.clone(),
            config_account.clone(),
            pyth_fee_vault.clone(),
            system_program_account.clone(),
        ],
        &[signer_seeds],
    )?;

    Ok(())
}

fn process_request_with_callback(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    entropy_data: &[u8],
) -> ProgramResult {
    let mut account_info_iter = accounts.iter();
    let requester_signer = next_account_info(&mut account_info_iter)?;
    let payer = next_account_info(&mut account_info_iter)?;
    let requester_program = next_account_info(&mut account_info_iter)?;
    let request_account = next_account_info(&mut account_info_iter)?;
    let provider_account = next_account_info(&mut account_info_iter)?;
    let provider_vault = next_account_info(&mut account_info_iter)?;
    let config_account = next_account_info(&mut account_info_iter)?;
    let pyth_fee_vault = next_account_info(&mut account_info_iter)?;
    let system_program_account = next_account_info(&mut account_info_iter)?;
    let callback_program = next_account_info(&mut account_info_iter)?;
    let entropy_program = next_account_info(&mut account_info_iter)?;

    if requester_program.key != program_id || callback_program.key != program_id {
        return Err(ProgramError::InvalidArgument);
    }

    let (expected_signer, bump) = Pubkey::find_program_address(
        &[REQUESTER_SIGNER_SEED, entropy_program.key.as_ref()],
        program_id,
    );
    if requester_signer.key != &expected_signer {
        return Err(ProgramError::InvalidSeeds);
    }

    let entropy_ix = Instruction {
        program_id: *entropy_program.key,
        data: entropy_data.to_vec(),
        accounts: vec![
            AccountMeta::new_readonly(*requester_signer.key, true),
            AccountMeta::new(*payer.key, true),
            AccountMeta::new_readonly(*requester_program.key, false),
            AccountMeta::new(*request_account.key, true),
            AccountMeta::new(*provider_account.key, false),
            AccountMeta::new(*provider_vault.key, false),
            AccountMeta::new_readonly(*config_account.key, false),
            AccountMeta::new(*pyth_fee_vault.key, false),
            AccountMeta::new_readonly(*system_program_account.key, false),
            AccountMeta::new_readonly(*callback_program.key, false),
        ],
    };

    let signer_seeds: &[&[u8]] = &[REQUESTER_SIGNER_SEED, entropy_program.key.as_ref(), &[bump]];
    invoke_signed(
        &entropy_ix,
        &[
            requester_signer.clone(),
            payer.clone(),
            requester_program.clone(),
            request_account.clone(),
            provider_account.clone(),
            provider_vault.clone(),
            config_account.clone(),
            pyth_fee_vault.clone(),
            system_program_account.clone(),
            callback_program.clone(),
        ],
        &[signer_seeds],
    )?;

    Ok(())
}

fn process_callback(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    if data.len() != 32 + 8 + 32 + 32 {
        return Err(ProgramError::InvalidInstructionData);
    }

    let mut account_info_iter = accounts.iter();
    let entropy_signer = next_account_info(&mut account_info_iter)?;
    let callback_state = next_account_info(&mut account_info_iter)?;

    if callback_state.owner != program_id || !callback_state.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }

    let entropy_program_id = Pubkey::new_from_array(
        data[..32]
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );
    let (expected_entropy_signer, _bump) =
        Pubkey::find_program_address(&[ENTROPY_SIGNER_SEED], &entropy_program_id);
    if entropy_signer.key != &expected_entropy_signer {
        return Err(ProgramError::InvalidSeeds);
    }

    let sequence_number = u64::from_le_bytes(
        data[32..40]
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );
    let provider = data[40..72]
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    let random_number = data[72..104]
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    let mut rand_bytes = [0u8; 8];
    rand_bytes.copy_from_slice(&random_number[..8]);
    let random_value = u64::from_le_bytes(rand_bytes) % 101;
    msg!("Random number (0-100): {}", random_value);

    let mut state_data = callback_state.try_borrow_mut_data()?;
    let state = bytemuck::from_bytes_mut::<CallbackState>(&mut state_data);
    state.sequence_number = sequence_number;
    state.provider = provider;
    state.random_number = random_number;
    state.called = 1;

    Ok(())
}
