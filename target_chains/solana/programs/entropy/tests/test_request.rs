mod test_utils;

use {
    bytemuck::{bytes_of, try_from_bytes},
    entropy::{
        accounts::{Provider, Request},
        constants::{CALLBACK_NOT_NECESSARY, REQUESTER_SIGNER_SEED},
        discriminator::{provider_discriminator, request_discriminator},
        error::EntropyError,
        instruction::{EntropyInstruction, RequestArgs},
        pda::{config_pda, provider_pda, provider_vault_pda, pyth_fee_vault_pda},
    },
    solana_program::{
        account_info::{next_account_info, AccountInfo},
        entrypoint::ProgramResult,
        hash::hashv,
        instruction::{AccountMeta, Instruction},
        program::invoke_signed,
        program_error::ProgramError,
        pubkey::Pubkey,
        system_program,
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

mod requester_program {
    use super::*;

    pub fn process_instruction(
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

        let signer_seeds: &[&[u8]] =
            &[REQUESTER_SIGNER_SEED, entropy_program.key.as_ref(), &[bump]];
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
}

fn build_requester_request_ix(
    requester_program_id: Pubkey,
    entropy_program_id: Pubkey,
    requester_signer: Pubkey,
    payer: Pubkey,
    request_account: Pubkey,
    provider_account: Pubkey,
    provider_vault: Pubkey,
    config: Pubkey,
    pyth_fee_vault: Pubkey,
    args: RequestArgs,
) -> Instruction {
    Instruction {
        program_id: requester_program_id,
        data: bytes_of(&args).to_vec(),
        accounts: vec![
            AccountMeta::new_readonly(requester_signer, false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(requester_program_id, false),
            AccountMeta::new(request_account, true),
            AccountMeta::new(provider_account, false),
            AccountMeta::new(provider_vault, false),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(pyth_fee_vault, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(entropy_program_id, false),
        ],
    }
}

async fn register_provider(
    banks_client: &mut solana_program_test::BanksClient,
    payer: &Keypair,
    program_id: Pubkey,
    fee_lamports: u64,
    chain_length: u64,
) -> (Pubkey, Pubkey) {
    let (provider_address, _) = provider_pda(&program_id, &payer.pubkey());
    let (provider_vault, _) = provider_vault_pda(&program_id, &payer.pubkey());
    let commitment = [7u8; 32];
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

fn new_program_test_with_requester(
    program_id: Pubkey,
    requester_program_id: Pubkey,
) -> ProgramTest {
    let mut program_test = new_entropy_program_test(program_id);
    program_test.add_program(
        "entropy-requester",
        requester_program_id,
        processor!(requester_program::process_instruction),
    );
    program_test
}

#[tokio::test]
async fn test_request_happy_path() {
    let program_id = Pubkey::new_unique();
    let requester_program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) =
        new_program_test_with_requester(program_id, requester_program_id)
            .start()
            .await;

    let pyth_fee_lamports = 321;
    initialize_config(&mut banks_client, &payer, program_id, pyth_fee_lamports).await;

    let provider_fee = 75;
    let (provider_address, provider_vault) =
        register_provider(&mut banks_client, &payer, program_id, provider_fee, 3).await;
    let (config_address, _) = config_pda(&program_id);
    let (pyth_fee_vault, _) = pyth_fee_vault_pda(&program_id);

    let (requester_signer, _) = Pubkey::find_program_address(
        &[REQUESTER_SIGNER_SEED, program_id.as_ref()],
        &requester_program_id,
    );

    let request_account = Keypair::new();
    let args = RequestArgs {
        user_commitment: [9u8; 32],
        use_blockhash: 1,
        _padding0: [0u8; 3],
        compute_unit_limit: 0,
    };

    let provider_vault_before = banks_client
        .get_account(provider_vault)
        .await
        .unwrap()
        .unwrap()
        .lamports;
    let pyth_fee_vault_before = banks_client
        .get_account(pyth_fee_vault)
        .await
        .unwrap()
        .unwrap()
        .lamports;

    let instruction = build_requester_request_ix(
        requester_program_id,
        program_id,
        requester_signer,
        payer.pubkey(),
        request_account.pubkey(),
        provider_address,
        provider_vault,
        config_address,
        pyth_fee_vault,
        args,
    );

    submit_tx(
        &mut banks_client,
        &payer,
        &[instruction],
        &[&request_account],
    )
    .await;

    let provider_account = banks_client
        .get_account(provider_address)
        .await
        .unwrap()
        .unwrap();
    let provider = try_from_bytes::<Provider>(&provider_account.data).unwrap();
    assert_eq!(provider.discriminator, provider_discriminator());
    assert_eq!(provider.sequence_number, 2);

    let provider_vault_after = banks_client
        .get_account(provider_vault)
        .await
        .unwrap()
        .unwrap()
        .lamports;
    assert_eq!(provider_vault_after - provider_vault_before, provider_fee);

    let pyth_fee_vault_after = banks_client
        .get_account(pyth_fee_vault)
        .await
        .unwrap()
        .unwrap()
        .lamports;
    assert_eq!(
        pyth_fee_vault_after - pyth_fee_vault_before,
        pyth_fee_lamports
    );

    let request_account_data = banks_client
        .get_account(request_account.pubkey())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request_account_data.owner, program_id);
    assert_eq!(request_account_data.data.len(), Request::LEN);
    assert_eq!(
        request_account_data.lamports,
        Rent::default().minimum_balance(Request::LEN)
    );

    let request = try_from_bytes::<Request>(&request_account_data.data).unwrap();
    assert_eq!(request.discriminator, request_discriminator());
    assert_eq!(request.provider, payer.pubkey().to_bytes());
    assert_eq!(request.sequence_number, 1);
    assert_eq!(request.num_hashes, 1);
    assert_eq!(
        request.requester_program_id,
        requester_program_id.to_bytes()
    );
    assert_eq!(request.use_blockhash, 1);
    assert_eq!(request.callback_status, CALLBACK_NOT_NECESSARY);
    assert_eq!(request.compute_unit_limit, 0);
    assert!(request.request_slot > 0);

    let expected_commitment =
        hashv(&[&args.user_commitment, &provider.current_commitment]).to_bytes();
    assert_eq!(request.commitment, expected_commitment);
}

#[tokio::test]
async fn test_request_out_of_randomness() {
    let program_id = Pubkey::new_unique();
    let requester_program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) =
        new_program_test_with_requester(program_id, requester_program_id)
            .start()
            .await;

    initialize_config(&mut banks_client, &payer, program_id, 0).await;

    let (provider_address, provider_vault) =
        register_provider(&mut banks_client, &payer, program_id, 1, 1).await;
    let (config_address, _) = config_pda(&program_id);
    let (pyth_fee_vault, _) = pyth_fee_vault_pda(&program_id);

    let (requester_signer, _) = Pubkey::find_program_address(
        &[REQUESTER_SIGNER_SEED, program_id.as_ref()],
        &requester_program_id,
    );

    let request_account = Keypair::new();
    let args = RequestArgs {
        user_commitment: [2u8; 32],
        use_blockhash: 0,
        _padding0: [0u8; 3],
        compute_unit_limit: 0,
    };

    let instruction = build_requester_request_ix(
        requester_program_id,
        program_id,
        requester_signer,
        payer.pubkey(),
        request_account.pubkey(),
        provider_address,
        provider_vault,
        config_address,
        pyth_fee_vault,
        args,
    );

    let err = submit_tx_expect_err(
        &mut banks_client,
        &payer,
        &[instruction],
        &[&request_account],
    )
    .await;

    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(EntropyError::OutOfRandomness as u32)
        )
    );
}

#[tokio::test]
async fn test_request_rejects_invalid_blockhash_flag() {
    let program_id = Pubkey::new_unique();
    let requester_program_id = Pubkey::new_unique();
    let (mut banks_client, payer, _) =
        new_program_test_with_requester(program_id, requester_program_id)
            .start()
            .await;

    initialize_config(&mut banks_client, &payer, program_id, 0).await;

    let (provider_address, provider_vault) =
        register_provider(&mut banks_client, &payer, program_id, 1, 3).await;
    let (config_address, _) = config_pda(&program_id);
    let (pyth_fee_vault, _) = pyth_fee_vault_pda(&program_id);

    let (requester_signer, _) = Pubkey::find_program_address(
        &[REQUESTER_SIGNER_SEED, program_id.as_ref()],
        &requester_program_id,
    );

    let request_account = Keypair::new();
    let args = RequestArgs {
        user_commitment: [2u8; 32],
        use_blockhash: 2,
        _padding0: [0u8; 3],
        compute_unit_limit: 0,
    };

    let instruction = build_requester_request_ix(
        requester_program_id,
        program_id,
        requester_signer,
        payer.pubkey(),
        request_account.pubkey(),
        provider_address,
        provider_vault,
        config_address,
        pyth_fee_vault,
        args,
    );

    let err = submit_tx_expect_err(
        &mut banks_client,
        &payer,
        &[instruction],
        &[&request_account],
    )
    .await;

    assert_eq!(
        err,
        TransactionError::InstructionError(0, InstructionError::InvalidInstructionData)
    );
}
