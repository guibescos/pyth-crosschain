use {
    bytemuck::{bytes_of, try_from_bytes},
    entropy::{
        accounts::Provider,
        constants::{COMMITMENT_METADATA_LEN, URI_LEN},
        discriminator::{config_discriminator, provider_discriminator},
        error::EntropyError,
        instruction::{EntropyInstruction, InitializeArgs, RegisterProviderArgs},
        pda::{config_pda, provider_pda, provider_vault_pda},
    },
    solana_program::{
        instruction::{AccountMeta, Instruction},
        pubkey::Pubkey,
        system_program,
    },
    solana_program_test::{processor, ProgramTest},
    solana_sdk::{
        account::Account,
        instruction::InstructionError,
        rent::Rent,
        signature::{Keypair, Signer},
        transaction::{Transaction, TransactionError},
    },
};

fn build_initialize_ix(
    program_id: Pubkey,
    payer: Pubkey,
    admin: Pubkey,
    default_provider: Pubkey,
    pyth_fee_lamports: u64,
) -> Instruction {
    let (config, _) = config_pda(&program_id);
    let (pyth_fee_vault, _) = entropy::pda::pyth_fee_vault_pda(&program_id);
    let args = InitializeArgs {
        admin: admin.to_bytes(),
        pyth_fee_lamports,
        default_provider: default_provider.to_bytes(),
    };
    let mut data = Vec::with_capacity(8 + core::mem::size_of::<InitializeArgs>());
    data.extend_from_slice(&EntropyInstruction::Initialize.discriminator());
    data.extend_from_slice(bytes_of(&args));

    Instruction {
        program_id,
        data,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(config, false),
            AccountMeta::new(pyth_fee_vault, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
    }
}

fn build_register_provider_ix(
    program_id: Pubkey,
    provider_authority: Pubkey,
    provider_account: Pubkey,
    provider_vault: Pubkey,
    config: Pubkey,
    args: RegisterProviderArgs,
    provider_authority_is_signer: bool,
) -> Instruction {
    let mut data = Vec::with_capacity(8 + core::mem::size_of::<RegisterProviderArgs>());
    data.extend_from_slice(&EntropyInstruction::RegisterProvider.discriminator());
    data.extend_from_slice(bytes_of(&args));

    Instruction {
        program_id,
        data,
        accounts: vec![
            AccountMeta::new(provider_authority, provider_authority_is_signer),
            AccountMeta::new(provider_account, false),
            AccountMeta::new(provider_vault, false),
            AccountMeta::new(config, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
    }
}

fn build_register_args(
    fee_lamports: u64,
    commitment: [u8; 32],
    chain_length: u64,
    commitment_metadata: &[u8],
    uri: &[u8],
) -> RegisterProviderArgs {
    assert!(commitment_metadata.len() <= COMMITMENT_METADATA_LEN);
    assert!(uri.len() <= URI_LEN);

    let mut commitment_metadata_buf = [0u8; COMMITMENT_METADATA_LEN];
    commitment_metadata_buf[..commitment_metadata.len()]
        .copy_from_slice(commitment_metadata);

    let mut uri_buf = [0u8; URI_LEN];
    uri_buf[..uri.len()].copy_from_slice(uri);

    RegisterProviderArgs {
        fee_lamports,
        commitment,
        commitment_metadata_len: commitment_metadata.len() as u16,
        _padding0: [0u8; 6],
        commitment_metadata: commitment_metadata_buf,
        chain_length,
        uri_len: uri.len() as u16,
        uri: uri_buf,
        _padding1: [0u8; 6],
    }
}

async fn initialize_config(
    banks_client: &mut solana_program_test::BanksClient,
    payer: &Keypair,
    program_id: Pubkey,
) {
    let instruction = build_initialize_ix(
        program_id,
        payer.pubkey(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        1234,
    );
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let mut transaction =
        Transaction::new_with_payer(&[instruction], Some(&payer.pubkey()));
    transaction.sign(&[payer], recent_blockhash);
    banks_client.process_transaction(transaction).await.unwrap();

    let (config_address, _) = config_pda(&program_id);
    let config_account = banks_client
        .get_account(config_address)
        .await
        .unwrap()
        .unwrap();
    let config = try_from_bytes::<entropy::accounts::Config>(&config_account.data).unwrap();
    assert_eq!(config.discriminator, config_discriminator());
}

#[tokio::test]
async fn test_register_provider_happy_path() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = ProgramTest::new(
        "entropy",
        program_id,
        processor!(entropy::processor::process_instruction),
    )
    .start()
    .await;

    initialize_config(&mut banks_client, &payer, program_id).await;

    let (provider_address, provider_bump) = provider_pda(&program_id, &payer.pubkey());
    let (provider_vault, _) = provider_vault_pda(&program_id, &payer.pubkey());
    let (config_address, _) = config_pda(&program_id);

    let commitment = [7u8; 32];
    let commitment_metadata = b"meta";
    let uri = b"https://example.com/provider";
    let args = build_register_args(42, commitment, 5, commitment_metadata, uri);

    let instruction = build_register_provider_ix(
        program_id,
        payer.pubkey(),
        provider_address,
        provider_vault,
        config_address,
        args,
        true,
    );
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let mut transaction =
        Transaction::new_with_payer(&[instruction], Some(&payer.pubkey()));
    transaction.sign(&[&payer], recent_blockhash);
    banks_client.process_transaction(transaction).await.unwrap();

    let provider_account = banks_client
        .get_account(provider_address)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(provider_account.owner, program_id);
    assert_eq!(provider_account.data.len(), Provider::LEN);
    assert_eq!(
        provider_account.lamports,
        Rent::default().minimum_balance(Provider::LEN)
    );

    let provider = try_from_bytes::<Provider>(&provider_account.data).unwrap();
    assert_eq!(provider.discriminator, provider_discriminator());
    assert_eq!(provider.provider_authority, payer.pubkey().to_bytes());
    assert_eq!(provider.fee_lamports, 42);
    assert_eq!(provider.accrued_fees_lamports, 0);
    assert_eq!(provider.original_commitment, commitment);
    assert_eq!(provider.current_commitment, commitment);
    assert_eq!(provider.original_commitment_sequence_number, 0);
    assert_eq!(provider.current_commitment_sequence_number, 0);
    assert_eq!(provider.sequence_number, 1);
    assert_eq!(provider.end_sequence_number, 5);
    assert_eq!(provider.commitment_metadata_len, commitment_metadata.len() as u16);
    assert_eq!(
        &provider.commitment_metadata[..commitment_metadata.len()],
        commitment_metadata
    );
    assert_eq!(provider.uri_len, uri.len() as u16);
    assert_eq!(&provider.uri[..uri.len()], uri);
    assert_eq!(provider.fee_manager, [0u8; 32]);
    assert_eq!(provider.max_num_hashes, 0);
    assert_eq!(provider.default_compute_unit_limit, 0);
    assert_eq!(provider.bump, provider_bump);

    let vault_account = banks_client
        .get_account(provider_vault)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vault_account.owner, system_program::id());
    assert_eq!(vault_account.data.len(), 0);
    assert!(vault_account.lamports >= Rent::default().minimum_balance(0));
}

#[tokio::test]
async fn test_register_provider_rotation_updates_commitment_and_sequence() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = ProgramTest::new(
        "entropy",
        program_id,
        processor!(entropy::processor::process_instruction),
    )
    .start()
    .await;

    initialize_config(&mut banks_client, &payer, program_id).await;

    let (provider_address, _) = provider_pda(&program_id, &payer.pubkey());
    let (provider_vault, _) = provider_vault_pda(&program_id, &payer.pubkey());
    let (config_address, _) = config_pda(&program_id);

    let first_commitment = [1u8; 32];
    let first_args = build_register_args(10, first_commitment, 3, b"meta-1", b"uri-1");
    let instruction = build_register_provider_ix(
        program_id,
        payer.pubkey(),
        provider_address,
        provider_vault,
        config_address,
        first_args,
        true,
    );
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let mut transaction =
        Transaction::new_with_payer(&[instruction], Some(&payer.pubkey()));
    transaction.sign(&[&payer], recent_blockhash);
    banks_client.process_transaction(transaction).await.unwrap();

    let provider_before = banks_client
        .get_account(provider_address)
        .await
        .unwrap()
        .unwrap();
    let provider_before_data = try_from_bytes::<Provider>(&provider_before.data).unwrap();
    let vault_before = banks_client
        .get_account(provider_vault)
        .await
        .unwrap()
        .unwrap();

    let second_commitment = [9u8; 32];
    let second_args =
        build_register_args(55, second_commitment, 4, b"meta-2", b"uri-2");
    let instruction = build_register_provider_ix(
        program_id,
        payer.pubkey(),
        provider_address,
        provider_vault,
        config_address,
        second_args,
        true,
    );
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let mut transaction =
        Transaction::new_with_payer(&[instruction], Some(&payer.pubkey()));
    transaction.sign(&[&payer], recent_blockhash);
    banks_client.process_transaction(transaction).await.unwrap();

    let provider_after = banks_client
        .get_account(provider_address)
        .await
        .unwrap()
        .unwrap();
    let provider = try_from_bytes::<Provider>(&provider_after.data).unwrap();
    assert_eq!(provider.sequence_number, provider_before_data.sequence_number + 1);
    assert_eq!(provider.end_sequence_number, provider_before_data.sequence_number + 4);
    assert_eq!(provider.original_commitment_sequence_number, provider_before_data.sequence_number);
    assert_eq!(provider.current_commitment_sequence_number, provider_before_data.sequence_number);
    assert_eq!(provider.original_commitment, second_commitment);
    assert_eq!(provider.current_commitment, second_commitment);
    assert_eq!(provider.accrued_fees_lamports, provider_before_data.accrued_fees_lamports);

    let vault_after = banks_client
        .get_account(provider_vault)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vault_after.lamports, vault_before.lamports);
    assert_eq!(vault_after.owner, vault_before.owner);
    assert_eq!(vault_after.data.len(), vault_before.data.len());
}

#[tokio::test]
async fn test_register_provider_rejects_zero_chain_length() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = ProgramTest::new(
        "entropy",
        program_id,
        processor!(entropy::processor::process_instruction),
    )
    .start()
    .await;

    initialize_config(&mut banks_client, &payer, program_id).await;

    let (provider_address, _) = provider_pda(&program_id, &payer.pubkey());
    let (provider_vault, _) = provider_vault_pda(&program_id, &payer.pubkey());
    let (config_address, _) = config_pda(&program_id);

    let args = build_register_args(1, [0u8; 32], 0, b"meta", b"uri");
    let instruction = build_register_provider_ix(
        program_id,
        payer.pubkey(),
        provider_address,
        provider_vault,
        config_address,
        args,
        true,
    );
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let mut transaction =
        Transaction::new_with_payer(&[instruction], Some(&payer.pubkey()));
    transaction.sign(&[&payer], recent_blockhash);
    let err = banks_client.process_transaction(transaction).await.unwrap_err();
    assert_eq!(
        err.unwrap(),
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[tokio::test]
async fn test_register_provider_requires_provider_authority_signer() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = ProgramTest::new(
        "entropy",
        program_id,
        processor!(entropy::processor::process_instruction),
    )
    .start()
    .await;

    initialize_config(&mut banks_client, &payer, program_id).await;

    let provider_authority = Pubkey::new_unique();
    let (provider_address, _) = provider_pda(&program_id, &provider_authority);
    let (provider_vault, _) = provider_vault_pda(&program_id, &provider_authority);
    let (config_address, _) = config_pda(&program_id);

    let args = build_register_args(1, [1u8; 32], 1, b"meta", b"uri");
    let instruction = build_register_provider_ix(
        program_id,
        provider_authority,
        provider_address,
        provider_vault,
        config_address,
        args,
        false,
    );
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let mut transaction =
        Transaction::new_with_payer(&[instruction], Some(&payer.pubkey()));
    transaction.sign(&[&payer], recent_blockhash);
    let err = banks_client.process_transaction(transaction).await.unwrap_err();
    assert_eq!(
        err.unwrap(),
        TransactionError::InstructionError(0, InstructionError::MissingRequiredSignature)
    );
}

#[tokio::test]
async fn test_register_provider_rejects_wrong_provider_pda() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = ProgramTest::new(
        "entropy",
        program_id,
        processor!(entropy::processor::process_instruction),
    )
    .start()
    .await;

    initialize_config(&mut banks_client, &payer, program_id).await;

    let provider_authority = payer.pubkey();
    let provider_address = Pubkey::new_unique();
    let (provider_vault, _) = provider_vault_pda(&program_id, &provider_authority);
    let (config_address, _) = config_pda(&program_id);

    let args = build_register_args(1, [2u8; 32], 2, b"meta", b"uri");
    let instruction = build_register_provider_ix(
        program_id,
        provider_authority,
        provider_address,
        provider_vault,
        config_address,
        args,
        true,
    );
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let mut transaction =
        Transaction::new_with_payer(&[instruction], Some(&payer.pubkey()));
    transaction.sign(&[&payer], recent_blockhash);
    let err = banks_client.process_transaction(transaction).await.unwrap_err();
    assert_eq!(
        err.unwrap(),
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(EntropyError::InvalidPda as u32)
        )
    );
}

#[tokio::test]
async fn test_register_provider_rejects_existing_provider_wrong_owner_or_size() {
    let program_id = Pubkey::new_unique();
    let provider_authority = Keypair::new();
    let (provider_address, _) = provider_pda(&program_id, &provider_authority.pubkey());

    let mut program_test = ProgramTest::new(
        "entropy",
        program_id,
        processor!(entropy::processor::process_instruction),
    );
    program_test.add_account(
        provider_address,
        Account {
            lamports: Rent::default().minimum_balance(1),
            data: vec![0u8; 1],
            owner: system_program::id(),
            executable: false,
            rent_epoch: 0,
        },
    );

    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_config(&mut banks_client, &payer, program_id).await;

    let (provider_vault, _) = provider_vault_pda(&program_id, &provider_authority.pubkey());
    let (config_address, _) = config_pda(&program_id);

    let args = build_register_args(1, [3u8; 32], 3, b"meta", b"uri");
    let instruction = build_register_provider_ix(
        program_id,
        provider_authority.pubkey(),
        provider_address,
        provider_vault,
        config_address,
        args,
        true,
    );
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let mut transaction =
        Transaction::new_with_payer(&[instruction], Some(&payer.pubkey()));
    transaction.sign(&[&payer, &provider_authority], recent_blockhash);
    let err = banks_client.process_transaction(transaction).await.unwrap_err();
    assert_eq!(
        err.unwrap(),
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(EntropyError::InvalidAccount as u32)
        )
    );
}
