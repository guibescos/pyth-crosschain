mod test_utils;

#[allow(deprecated)]
use {
    bytemuck::{bytes_of, cast_slice, try_from_bytes, Pod, Zeroable},
    entropy::{
        accounts::{CallbackMeta, Provider, Request},
        constants::{CALLBACK_NOT_STARTED, REQUESTER_SIGNER_SEED},
        discriminator::{provider_discriminator, request_discriminator},
        error::EntropyError,
        instruction::{EntropyInstruction, RevealArgs},
        pda::{
            config_pda, entropy_signer_pda, provider_pda, provider_vault_pda, pyth_fee_vault_pda,
        },
    },
    simple_requester::{
        CallbackState, CALLBACK_ACTION, CALLBACK_STATE_LEN, REQUEST_WITH_CALLBACK_ACTION,
    },
    solana_program::{
        hash::{hash, hashv},
        instruction::{AccountMeta, Instruction},
        pubkey::Pubkey,
        system_instruction, system_program,
        sysvar::slot_hashes,
    },
    solana_program_test::{processor, ProgramTest},
    solana_sdk::{
        instruction::InstructionError,
        rent::Rent,
        signature::{Keypair, Signer},
        transaction::TransactionError,
    },
    test_utils::{
        build_register_args, build_register_provider_ix, initialize_config,
        new_entropy_program_test, submit_tx, submit_tx_expect_err,
    },
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RequestWithCallbackHeader {
    user_randomness: [u8; 32],
    compute_unit_limit: u32,
    callback_accounts_len: u32,
}

fn new_program_test_with_requester(
    program_id: Pubkey,
    requester_program_id: Pubkey,
) -> ProgramTest {
    let mut program_test = new_entropy_program_test(program_id);
    program_test.add_program(
        "entropy-requester",
        requester_program_id,
        processor!(simple_requester::process_instruction),
    );
    program_test
}

async fn register_provider(
    banks_client: &mut solana_program_test::BanksClient,
    payer: &Keypair,
    program_id: Pubkey,
    fee_lamports: u64,
    chain_length: u64,
    commitment: [u8; 32],
) -> (Pubkey, Pubkey) {
    let (provider_address, _) = provider_pda(&program_id, &payer.pubkey());
    let (provider_vault, _) = provider_vault_pda(&program_id, &payer.pubkey());
    let args = build_register_args(fee_lamports, commitment, chain_length);

    let instruction = build_register_provider_ix(
        program_id,
        payer.pubkey(),
        provider_address,
        provider_vault,
        args,
        true,
    );
    submit_tx(banks_client, payer, &[instruction], &[]).await;

    (provider_address, provider_vault)
}

fn build_request_with_callback_data(
    user_randomness: [u8; 32],
    compute_unit_limit: u32,
    callback_accounts: &[CallbackMeta],
    callback_ix_data: &[u8],
) -> Vec<u8> {
    let header = RequestWithCallbackHeader {
        user_randomness,
        compute_unit_limit,
        callback_accounts_len: callback_accounts.len() as u32,
    };

    let mut data = Vec::with_capacity(
        8 + core::mem::size_of::<RequestWithCallbackHeader>()
            + callback_accounts.len() * CallbackMeta::LEN
            + 4
            + callback_ix_data.len(),
    );
    data.extend_from_slice(&EntropyInstruction::RequestWithCallback.discriminator());
    data.extend_from_slice(bytes_of(&header));
    data.extend_from_slice(cast_slice(callback_accounts));
    data.extend_from_slice(&(callback_ix_data.len() as u32).to_le_bytes());
    data.extend_from_slice(callback_ix_data);
    data
}

#[tokio::test]
async fn test_request_with_callback_rejects_entropy_program_in_callback_accounts() {
    let program_id = Pubkey::new_unique();
    let requester_program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) =
        new_program_test_with_requester(program_id, requester_program_id)
            .start()
            .await;

    initialize_config(&mut banks_client, &payer, program_id, 0).await;

    let commitment = hash(&[7u8; 32]).to_bytes();
    let (provider_address, _provider_vault) =
        register_provider(&mut banks_client, &payer, program_id, 1, 3, commitment).await;

    let (config_address, _) = config_pda(&program_id);
    let (pyth_fee_vault, _) = pyth_fee_vault_pda(&program_id);

    let request_account = Keypair::new();
    let callback_accounts = [CallbackMeta {
        pubkey: program_id.to_bytes(),
        is_signer: 0,
        is_writable: 0,
    }];

    let entropy_request_data =
        build_request_with_callback_data([9u8; 32], 200_000, &callback_accounts, &[]);

    let mut requester_data = Vec::with_capacity(1 + entropy_request_data.len());
    requester_data.push(REQUEST_WITH_CALLBACK_ACTION);
    requester_data.extend_from_slice(&entropy_request_data);

    let (requester_signer, _) = Pubkey::find_program_address(
        &[REQUESTER_SIGNER_SEED, program_id.as_ref()],
        &requester_program_id,
    );

    let request_with_callback_ix = Instruction {
        program_id: requester_program_id,
        data: requester_data,
        accounts: vec![
            AccountMeta::new_readonly(requester_signer, false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(requester_program_id, false),
            AccountMeta::new(request_account.pubkey(), true),
            AccountMeta::new(provider_address, false),
            AccountMeta::new(provider_vault_pda(&program_id, &payer.pubkey()).0, false),
            AccountMeta::new_readonly(config_address, false),
            AccountMeta::new(pyth_fee_vault, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(requester_program_id, false),
            AccountMeta::new_readonly(program_id, false),
        ],
    };

    let err = submit_tx_expect_err(
        &mut banks_client,
        &payer,
        &[request_with_callback_ix],
        &[&request_account],
    )
    .await;

    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(EntropyError::InvalidAccount as u32)
        )
    );
}

#[tokio::test]
async fn test_reveal_with_callback_flow() {
    let program_id = Pubkey::new_unique();
    let requester_program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) =
        new_program_test_with_requester(program_id, requester_program_id)
            .start()
            .await;

    initialize_config(&mut banks_client, &payer, program_id, 0).await;

    let provider_contribution = [7u8; 32];
    let commitment = hash(&provider_contribution).to_bytes();

    let (provider_address, _provider_vault) =
        register_provider(&mut banks_client, &payer, program_id, 1, 3, commitment).await;

    let (config_address, _) = config_pda(&program_id);
    let (pyth_fee_vault, _) = pyth_fee_vault_pda(&program_id);
    let (entropy_signer, _) = entropy_signer_pda(&program_id);

    let callback_state = Keypair::new();
    let create_callback_state_ix = system_instruction::create_account(
        &payer.pubkey(),
        &callback_state.pubkey(),
        Rent::default().minimum_balance(CALLBACK_STATE_LEN),
        CALLBACK_STATE_LEN as u64,
        &requester_program_id,
    );
    submit_tx(
        &mut banks_client,
        &payer,
        &[create_callback_state_ix],
        &[&callback_state],
    )
    .await;

    let request_account = Keypair::new();
    let user_randomness = [9u8; 32];
    let compute_unit_limit = 200_000;

    let callback_accounts = [CallbackMeta {
        pubkey: callback_state.pubkey().to_bytes(),
        is_signer: 0,
        is_writable: 1,
    }];

    let mut callback_ix_data = Vec::with_capacity(1 + 32);
    callback_ix_data.push(CALLBACK_ACTION);
    callback_ix_data.extend_from_slice(program_id.as_ref());

    let entropy_request_data = build_request_with_callback_data(
        user_randomness,
        compute_unit_limit,
        &callback_accounts,
        &callback_ix_data,
    );

    let mut requester_data = Vec::with_capacity(1 + entropy_request_data.len());
    requester_data.push(REQUEST_WITH_CALLBACK_ACTION);
    requester_data.extend_from_slice(&entropy_request_data);

    let (requester_signer, _) = Pubkey::find_program_address(
        &[REQUESTER_SIGNER_SEED, program_id.as_ref()],
        &requester_program_id,
    );

    let request_with_callback_ix = Instruction {
        program_id: requester_program_id,
        data: requester_data,
        accounts: vec![
            AccountMeta::new_readonly(requester_signer, false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(requester_program_id, false),
            AccountMeta::new(request_account.pubkey(), true),
            AccountMeta::new(provider_address, false),
            AccountMeta::new(provider_vault_pda(&program_id, &payer.pubkey()).0, false),
            AccountMeta::new_readonly(config_address, false),
            AccountMeta::new(pyth_fee_vault, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(requester_program_id, false),
            AccountMeta::new_readonly(program_id, false),
        ],
    };

    submit_tx(
        &mut banks_client,
        &payer,
        &[request_with_callback_ix],
        &[&request_account],
    )
    .await;

    let request_account_data = banks_client
        .get_account(request_account.pubkey())
        .await
        .unwrap()
        .unwrap();
    let request = try_from_bytes::<Request>(&request_account_data.data).unwrap();
    assert_eq!(request.discriminator, request_discriminator());
    assert_eq!(request.callback_status, CALLBACK_NOT_STARTED);

    let reveal_args = RevealArgs {
        user_contribution: user_randomness,
        provider_contribution,
    };
    let mut reveal_data = Vec::with_capacity(8 + core::mem::size_of::<RevealArgs>());
    reveal_data.extend_from_slice(&EntropyInstruction::RevealWithCallback.discriminator());
    reveal_data.extend_from_slice(bytes_of(&reveal_args));

    let reveal_ix = Instruction {
        program_id,
        data: reveal_data,
        accounts: vec![
            AccountMeta::new(request_account.pubkey(), false),
            AccountMeta::new(provider_address, false),
            AccountMeta::new_readonly(slot_hashes::id(), false),
            AccountMeta::new_readonly(entropy_signer, false),
            AccountMeta::new_readonly(requester_program_id, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(callback_state.pubkey(), false),
        ],
    };

    submit_tx(&mut banks_client, &payer, &[reveal_ix], &[]).await;

    let callback_state_account = banks_client
        .get_account(callback_state.pubkey())
        .await
        .unwrap()
        .unwrap();
    let callback_state = bytemuck::from_bytes::<CallbackState>(&callback_state_account.data);

    let expected_random = hashv(&[&user_randomness, &provider_contribution, &[0u8; 32]]).to_bytes();
    assert_eq!(callback_state.called, 1);
    assert_eq!(callback_state.sequence_number, 1);
    assert_eq!(callback_state.provider, payer.pubkey().to_bytes());
    assert_eq!(callback_state.random_number, expected_random);

    let provider_account = banks_client
        .get_account(provider_address)
        .await
        .unwrap()
        .unwrap();
    let provider = try_from_bytes::<Provider>(&provider_account.data).unwrap();
    assert_eq!(provider.discriminator, provider_discriminator());
    assert_eq!(provider.current_commitment_sequence_number, 1);
    assert_eq!(provider.current_commitment, provider_contribution);

    assert!(banks_client
        .get_account(request_account.pubkey())
        .await
        .unwrap()
        .is_none());
}
