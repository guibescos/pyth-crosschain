mod test_utils;

use {
    bytemuck::try_from_bytes,
    entropy::{
        accounts::Config,
        discriminator::config_discriminator,
        pda::{config_pda, pyth_fee_vault_pda},
    },
    solana_program::{pubkey::Pubkey, system_program},
    solana_sdk::{
        instruction::InstructionError, rent::Rent, signature::Signer, transaction::TransactionError,
    },
    test_utils::{build_initialize_ix, new_entropy_program_test, submit_tx, submit_tx_expect_err},
};

#[tokio::test]
async fn test_initialize_happy_path() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = new_entropy_program_test(program_id).start().await;

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
    submit_tx(&mut banks_client, &payer, &[instruction], &[]).await;

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
async fn test_initialize_records_prefunded_fee_vault() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = new_entropy_program_test(program_id).start().await;

    let (fee_vault_address, _) = pyth_fee_vault_pda(&program_id);
    let pre_fund_lamports = Rent::default().minimum_balance(0) + 42;

    let prefund_ix = solana_sdk::system_instruction::transfer(
        &payer.pubkey(),
        &fee_vault_address,
        pre_fund_lamports,
    );
    submit_tx(&mut banks_client, &payer, &[prefund_ix], &[]).await;

    let instruction = build_initialize_ix(
        program_id,
        payer.pubkey(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        1234,
    );
    submit_tx(&mut banks_client, &payer, &[instruction], &[]).await;

    let (config_address, _) = config_pda(&program_id);
    let config_account = banks_client
        .get_account(config_address)
        .await
        .unwrap()
        .unwrap();

    let fee_vault_account = banks_client
        .get_account(fee_vault_address)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fee_vault_account.owner, system_program::id());
    assert_eq!(fee_vault_account.data.len(), 0);
    assert_eq!(fee_vault_account.lamports, pre_fund_lamports);
}

#[tokio::test]
async fn test_initialize_rejects_zero_admin() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = new_entropy_program_test(program_id).start().await;

    let instruction = build_initialize_ix(
        program_id,
        payer.pubkey(),
        Pubkey::default(),
        Pubkey::new_unique(),
        1,
    );
    let err = submit_tx_expect_err(&mut banks_client, &payer, &[instruction], &[]).await;
    assert_eq!(
        err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[tokio::test]
async fn test_initialize_rejects_zero_default_provider() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = new_entropy_program_test(program_id).start().await;

    let instruction = build_initialize_ix(
        program_id,
        payer.pubkey(),
        Pubkey::new_unique(),
        Pubkey::default(),
        1,
    );
    let err = submit_tx_expect_err(&mut banks_client, &payer, &[instruction], &[]).await;
    assert_eq!(
        err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}
