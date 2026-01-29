use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    hash::hash,
    program::set_return_data,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_program,
};

use crate::{
    accounts::{CallbackMeta, Config, Provider, Request},
    constants::{
        CALLBACK_IX_DATA_LEN, CALLBACK_NOT_STARTED, MAX_CALLBACK_ACCOUNTS, REQUESTER_SIGNER_SEED,
    },
    error::EntropyError,
    instruction::RequestArgs,
    pda::{config_pda, provider_pda, provider_vault_pda, pyth_fee_vault_pda},
    pda_loader::{load_account, load_account_mut},
    processor::request::request_helper,
};

pub fn process_request_with_callback(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let args = parse_request_with_callback_args(data)?;

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

    if !requester_signer.is_signer || !payer.is_signer || !request_account.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    if !payer.is_writable
        || !request_account.is_writable
        || !provider_account.is_writable
        || !provider_vault.is_writable
        || !pyth_fee_vault.is_writable
    {
        return Err(EntropyError::InvalidAccount.into());
    }

    if system_program_account.key != &system_program::ID {
        return Err(EntropyError::InvalidAccount.into());
    }

    if callback_program.key != requester_program.key {
        return Err(EntropyError::InvalidAccount.into());
    }

    let requester_signer_seed = [REQUESTER_SIGNER_SEED, program_id.as_ref()];
    let (expected_requester_signer, _bump) =
        Pubkey::find_program_address(&requester_signer_seed, requester_program.key);
    if requester_signer.key != &expected_requester_signer {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_config, _config_bump) = config_pda(program_id);
    if config_account.key != &expected_config {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_pyth_fee_vault, _pyth_fee_vault_bump) = pyth_fee_vault_pda(program_id);
    if pyth_fee_vault.key != &expected_pyth_fee_vault {
        return Err(EntropyError::InvalidPda.into());
    }
    if pyth_fee_vault.owner != &system_program::ID || pyth_fee_vault.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    if request_account.owner != &system_program::ID || request_account.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let config = load_account::<Config>(config_account, program_id)?;
    let mut provider = load_account_mut::<Provider>(provider_account, program_id)?;
    let provider_authority = Pubkey::new_from_array(provider.provider_authority);
    let (expected_provider, _provider_bump) = provider_pda(program_id, &provider_authority);
    if provider_account.key != &expected_provider {
        return Err(EntropyError::InvalidPda.into());
    }

    let (expected_provider_vault, _provider_vault_bump) =
        provider_vault_pda(program_id, &provider_authority);
    if provider_vault.key != &expected_provider_vault {
        return Err(EntropyError::InvalidPda.into());
    }
    if provider_vault.owner != &system_program::ID || provider_vault.data_len() != 0 {
        return Err(EntropyError::InvalidAccount.into());
    }

    let user_commitment = hash(&args.user_randomness).to_bytes();
    let request_args = RequestArgs {
        user_commitment,
        use_blockhash: 0,
        _padding0: [0u8; 3],
        compute_unit_limit: args.compute_unit_limit,
    };

    let sequence_number = request_helper(
        program_id,
        &request_args,
        &config,
        &mut provider,
        payer,
        requester_program,
        request_account,
        provider_vault,
        pyth_fee_vault,
        system_program_account,
    )?;

    {
        let mut request = load_account_mut::<Request>(request_account, program_id)?;
        request.callback_status = CALLBACK_NOT_STARTED;
        request.compute_unit_limit = args.compute_unit_limit;
        request.callback_accounts_len = args.callback_accounts.len() as u8;
        request.callback_ix_data_len = args.callback_ix_data.len() as u16;

        for (index, meta) in args.callback_accounts.iter().enumerate() {
            request.callback_accounts[index] = *meta;
        }
        for meta in request
            .callback_accounts
            .iter_mut()
            .skip(args.callback_accounts.len())
        {
            *meta = CallbackMeta {
                pubkey: [0u8; 32],
                is_signer: 0,
                is_writable: 0,
            };
        }

        let data_len = args.callback_ix_data.len();
        request.callback_ix_data[..data_len].copy_from_slice(&args.callback_ix_data);
        for byte in request.callback_ix_data.iter_mut().skip(data_len) {
            *byte = 0;
        }
    }

    set_return_data(&sequence_number.to_le_bytes());
    Ok(())
}

struct RequestWithCallbackArgs {
    user_randomness: [u8; 32],
    compute_unit_limit: u32,
    callback_accounts: Vec<CallbackMeta>,
    callback_ix_data: Vec<u8>,
}

fn parse_request_with_callback_args(data: &[u8]) -> Result<RequestWithCallbackArgs, ProgramError> {
    let mut cursor = data;

    let user_randomness = read_array_32(&mut cursor)?;
    let compute_unit_limit = read_u32(&mut cursor)?;
    let callback_accounts_len = read_u32(&mut cursor)? as usize;
    if callback_accounts_len > MAX_CALLBACK_ACCOUNTS {
        return Err(ProgramError::InvalidInstructionData);
    }

    let mut callback_accounts = Vec::with_capacity(callback_accounts_len);
    for _ in 0..callback_accounts_len {
        callback_accounts.push(read_callback_meta(&mut cursor)?);
    }

    let callback_ix_data_len = read_u32(&mut cursor)? as usize;
    if callback_ix_data_len > CALLBACK_IX_DATA_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let callback_ix_data = read_bytes(&mut cursor, callback_ix_data_len)?.to_vec();

    if !cursor.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }

    Ok(RequestWithCallbackArgs {
        user_randomness,
        compute_unit_limit,
        callback_accounts,
        callback_ix_data,
    })
}

fn read_callback_meta(data: &mut &[u8]) -> Result<CallbackMeta, ProgramError> {
    let pubkey = read_array_32(data)?;
    let is_signer = read_u8(data)?;
    let is_writable = read_u8(data)?;
    if is_signer > 1 || is_writable > 1 {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(CallbackMeta {
        pubkey,
        is_signer,
        is_writable,
    })
}

fn read_u32(data: &mut &[u8]) -> Result<u32, ProgramError> {
    let bytes = read_bytes(data, 4)?;
    Ok(u32::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3],
    ]))
}

fn read_u8(data: &mut &[u8]) -> Result<u8, ProgramError> {
    let bytes = read_bytes(data, 1)?;
    Ok(bytes[0])
}

fn read_array_32(data: &mut &[u8]) -> Result<[u8; 32], ProgramError> {
    let bytes = read_bytes(data, 32)?;
    let mut output = [0u8; 32];
    output.copy_from_slice(bytes);
    Ok(output)
}

fn read_bytes<'a>(data: &mut &'a [u8], len: usize) -> Result<&'a [u8], ProgramError> {
    if data.len() < len {
        return Err(ProgramError::InvalidInstructionData);
    }
    let (head, tail) = data.split_at(len);
    *data = tail;
    Ok(head)
}
