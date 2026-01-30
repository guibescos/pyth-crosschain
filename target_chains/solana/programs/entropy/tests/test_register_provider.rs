mod test_utils;

use {
    crate::test_utils::register_args::build_register_args_with_metadata,
    bytemuck::try_from_bytes,
    entropy::{
        accounts::Provider,
        discriminator::provider_discriminator,
        error::EntropyError,
        pda::{provider_pda, provider_vault_pda},
    },
    solana_program::{pubkey::Pubkey, system_program},
    solana_sdk::{
        account::Account,
        instruction::InstructionError,
        rent::Rent,
        signature::{Keypair, Signer},
        transaction::TransactionError,
    },
    test_utils::{
        build_register_provider_ix, initialize_config, new_entropy_program_test, submit_tx,
        submit_tx_expect_err,
    },
};

#[tokio::test]
async fn test_register_provider_happy_path() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = new_entropy_program_test(program_id).start().await;

    initialize_config(&mut banks_client, &payer, program_id, 1234).await;

    let (provider_address, provider_bump) = provider_pda(&program_id, &payer.pubkey());
    let (provider_vault, _) = provider_vault_pda(&program_id, &payer.pubkey());
    let commitment = [7u8; 32];
    let commitment_metadata = b"meta";
    let uri = b"https://example.com/provider";
    let args = build_register_args_with_metadata(42, commitment, 5, commitment_metadata, uri);

    let instruction = build_register_provider_ix(
        program_id,
        payer.pubkey(),
        provider_address,
        provider_vault,
        args,
        true,
    );
    submit_tx(&mut banks_client, &payer, &[instruction], &[]).await;

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
    assert_eq!(provider.original_commitment, commitment);
    assert_eq!(provider.current_commitment, commitment);
    assert_eq!(provider.original_commitment_sequence_number, 0);
    assert_eq!(provider.current_commitment_sequence_number, 0);
    assert_eq!(provider.sequence_number, 1);
    assert_eq!(provider.end_sequence_number, 5);
    assert_eq!(
        provider.commitment_metadata_len,
        commitment_metadata.len() as u16
    );
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
    let (mut banks_client, payer, _) = new_entropy_program_test(program_id).start().await;

    initialize_config(&mut banks_client, &payer, program_id, 1234).await;

    let (provider_address, _) = provider_pda(&program_id, &payer.pubkey());
    let (provider_vault, _) = provider_vault_pda(&program_id, &payer.pubkey());
    let first_commitment = [1u8; 32];
    let first_args =
        build_register_args_with_metadata(10, first_commitment, 3, b"meta-1", b"uri-1");
    let instruction = build_register_provider_ix(
        program_id,
        payer.pubkey(),
        provider_address,
        provider_vault,
        first_args,
        true,
    );
    submit_tx(&mut banks_client, &payer, &[instruction], &[]).await;

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
        build_register_args_with_metadata(55, second_commitment, 4, b"meta-2", b"uri-2");
    let instruction = build_register_provider_ix(
        program_id,
        payer.pubkey(),
        provider_address,
        provider_vault,
        second_args,
        true,
    );
    submit_tx(&mut banks_client, &payer, &[instruction], &[]).await;

    let provider_after = banks_client
        .get_account(provider_address)
        .await
        .unwrap()
        .unwrap();
    let provider = try_from_bytes::<Provider>(&provider_after.data).unwrap();
    assert_eq!(
        provider.sequence_number,
        provider_before_data.sequence_number + 1
    );
    assert_eq!(
        provider.end_sequence_number,
        provider_before_data.sequence_number + 4
    );
    assert_eq!(
        provider.original_commitment_sequence_number,
        provider_before_data.sequence_number
    );
    assert_eq!(
        provider.current_commitment_sequence_number,
        provider_before_data.sequence_number
    );
    assert_eq!(provider.original_commitment, second_commitment);
    assert_eq!(provider.current_commitment, second_commitment);
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
    let (mut banks_client, payer, _) = new_entropy_program_test(program_id).start().await;

    initialize_config(&mut banks_client, &payer, program_id, 1234).await;

    let (provider_address, _) = provider_pda(&program_id, &payer.pubkey());
    let (provider_vault, _) = provider_vault_pda(&program_id, &payer.pubkey());
    let args = build_register_args_with_metadata(1, [0u8; 32], 0, b"meta", b"uri");
    let instruction = build_register_provider_ix(
        program_id,
        payer.pubkey(),
        provider_address,
        provider_vault,
        args,
        true,
    );
    let err = submit_tx_expect_err(&mut banks_client, &payer, &[instruction], &[]).await;
    assert_eq!(
        err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[tokio::test]
async fn test_register_provider_requires_provider_authority_signer() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = new_entropy_program_test(program_id).start().await;

    initialize_config(&mut banks_client, &payer, program_id, 1234).await;

    let provider_authority = Pubkey::new_unique();
    let (provider_address, _) = provider_pda(&program_id, &provider_authority);
    let (provider_vault, _) = provider_vault_pda(&program_id, &provider_authority);
    let args = build_register_args_with_metadata(1, [1u8; 32], 1, b"meta", b"uri");
    let instruction = build_register_provider_ix(
        program_id,
        provider_authority,
        provider_address,
        provider_vault,
        args,
        false,
    );
    let err = submit_tx_expect_err(&mut banks_client, &payer, &[instruction], &[]).await;
    assert_eq!(
        err,
        TransactionError::InstructionError(0, InstructionError::MissingRequiredSignature)
    );
}

#[tokio::test]
async fn test_register_provider_rejects_wrong_provider_pda() {
    let program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) = new_entropy_program_test(program_id).start().await;

    initialize_config(&mut banks_client, &payer, program_id, 1234).await;

    let provider_authority = payer.pubkey();
    let provider_address = Pubkey::new_unique();
    let (provider_vault, _) = provider_vault_pda(&program_id, &provider_authority);
    let args = build_register_args_with_metadata(1, [2u8; 32], 2, b"meta", b"uri");
    let instruction = build_register_provider_ix(
        program_id,
        provider_authority,
        provider_address,
        provider_vault,
        args,
        true,
    );
    let err = submit_tx_expect_err(&mut banks_client, &payer, &[instruction], &[]).await;
    assert_eq!(
        err,
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

    let mut program_test = new_entropy_program_test(program_id);
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

    initialize_config(&mut banks_client, &payer, program_id, 1234).await;

    let (provider_vault, _) = provider_vault_pda(&program_id, &provider_authority.pubkey());
    let args = build_register_args_with_metadata(1, [3u8; 32], 3, b"meta", b"uri");
    let instruction = build_register_provider_ix(
        program_id,
        provider_authority.pubkey(),
        provider_address,
        provider_vault,
        args,
        true,
    );
    let err = submit_tx_expect_err(
        &mut banks_client,
        &payer,
        &[instruction],
        &[&provider_authority],
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
