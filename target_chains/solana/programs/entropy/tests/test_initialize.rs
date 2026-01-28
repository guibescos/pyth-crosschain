use {
    bytemuck::try_from_bytes,
    entropy::{
        accounts::Config,
        discriminator::config_discriminator,
        instruction::{EntropyInstruction, InitializeArgs},
        pda::{config_pda, pyth_fee_vault_pda},
    },
    solana_program::{
        instruction::{AccountMeta, Instruction},
        pubkey::Pubkey,
        system_program,
    },
    solana_program_test::{processor, BanksClientError, ProgramTest},
    solana_sdk::{
        instruction::InstructionError,
        rent::Rent,
        signature::Signer,
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
    let (pyth_fee_vault, _) = pyth_fee_vault_pda(&program_id);
    let args = InitializeArgs {
        admin: admin.to_bytes(),
        pyth_fee_lamports,
        default_provider: default_provider.to_bytes(),
    };
    let mut data = vec![EntropyInstruction::Initialize as u8];
    data.extend_from_slice(bytemuck::bytes_of(&args));

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

#[tokio::test]
async fn test_initialize_happy_path() {
    let program_id = Pubkey::new_unique();
    let (banks_client, payer, recent_blockhash) = ProgramTest::new(
        "entropy",
        program_id,
        processor!(entropy::processor::process_instruction),
    )
    .start()
    .await;

    let admin = Pubkey::new_unique();
    let default_provider = Pubkey::new_unique();
    let pyth_fee_lamports = 1234;

    let instruction = build_initialize_ix(
        program_id,
        payer.pubkey(),
        admin,
        default_provider,
        pyth_fee_lamports,
    );
    let mut transaction =
        Transaction::new_with_payer(&[instruction], Some(&payer.pubkey()));
    transaction.sign(&[&payer], recent_blockhash);
    banks_client.process_transaction(transaction).await.unwrap();

    let (config_address, expected_bump) = config_pda(&program_id);
    let (fee_vault_address, _) = pyth_fee_vault_pda(&program_id);

    let config_account = banks_client
        .get_account(config_address)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(config_account.owner, program_id);
    assert_eq!(config_account.data.len(), Config::LEN);
    let config = try_from_bytes::<Config>(&config_account.data).unwrap();
    assert_eq!(config.discriminator, config_discriminator());
    assert_eq!(config.admin, admin.to_bytes());
    assert_eq!(config.pyth_fee_lamports, pyth_fee_lamports);
    assert_eq!(config.accrued_pyth_fees_lamports, 0);
    assert_eq!(config.default_provider, default_provider.to_bytes());
    assert_eq!(config.proposed_admin, [0u8; 32]);
    assert_eq!(config.seed, [0u8; 32]);
    assert_eq!(config.bump, expected_bump);

    let fee_vault_account = banks_client
        .get_account(fee_vault_address)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fee_vault_account.owner, system_program::id());
    assert_eq!(fee_vault_account.data.len(), 0);
    assert!(fee_vault_account.lamports >= Rent::default().minimum_balance(0));
}

#[tokio::test]
async fn test_initialize_rejects_zero_admin() {
    let program_id = Pubkey::new_unique();
    let (banks_client, payer, recent_blockhash) = ProgramTest::new(
        "entropy",
        program_id,
        processor!(entropy::processor::process_instruction),
    )
    .start()
    .await;

    let instruction = build_initialize_ix(
        program_id,
        payer.pubkey(),
        Pubkey::default(),
        Pubkey::new_unique(),
        1,
    );
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
async fn test_initialize_rejects_zero_default_provider() {
    let program_id = Pubkey::new_unique();
    let (banks_client, payer, recent_blockhash) = ProgramTest::new(
        "entropy",
        program_id,
        processor!(entropy::processor::process_instruction),
    )
    .start()
    .await;

    let instruction = build_initialize_ix(
        program_id,
        payer.pubkey(),
        Pubkey::new_unique(),
        Pubkey::default(),
        1,
    );
    let mut transaction =
        Transaction::new_with_payer(&[instruction], Some(&payer.pubkey()));
    transaction.sign(&[&payer], recent_blockhash);
    let err = banks_client.process_transaction(transaction).await.unwrap_err();
    assert_eq!(
        err.unwrap(),
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}
