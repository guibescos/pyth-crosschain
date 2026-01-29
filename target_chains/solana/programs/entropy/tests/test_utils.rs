use {
    bytemuck::{bytes_of, try_from_bytes},
    entropy::{
        accounts::Config,
        constants::{COMMITMENT_METADATA_LEN, URI_LEN},
        discriminator::config_discriminator,
        instruction::{EntropyInstruction, InitializeArgs, RegisterProviderArgs},
        pda::{config_pda, pyth_fee_vault_pda},
    },
    solana_program::{
        instruction::{AccountMeta, Instruction},
        pubkey::Pubkey,
        system_program,
    },
    solana_program_test::BanksClient,
    solana_sdk::{
        signature::{Keypair, Signer},
        transaction::Transaction,
    },
};

pub fn build_initialize_ix(
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

pub fn build_register_provider_ix(
    program_id: Pubkey,
    provider_authority: Pubkey,
    provider_account: Pubkey,
    provider_vault: Pubkey,
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
            AccountMeta::new_readonly(system_program::id(), false),
        ],
    }
}

pub fn build_register_args_with_metadata(
    fee_lamports: u64,
    commitment: [u8; 32],
    chain_length: u64,
    commitment_metadata: &[u8],
    uri: &[u8],
) -> RegisterProviderArgs {
    assert!(commitment_metadata.len() <= COMMITMENT_METADATA_LEN);
    assert!(uri.len() <= URI_LEN);

    let mut commitment_metadata_buf = [0u8; COMMITMENT_METADATA_LEN];
    commitment_metadata_buf[..commitment_metadata.len()].copy_from_slice(commitment_metadata);

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

pub fn build_register_args(
    fee_lamports: u64,
    commitment: [u8; 32],
    chain_length: u64,
) -> RegisterProviderArgs {
    build_register_args_with_metadata(fee_lamports, commitment, chain_length, &[], &[])
}

pub async fn initialize_config(
    banks_client: &mut BanksClient,
    payer: &Keypair,
    program_id: Pubkey,
    pyth_fee_lamports: u64,
) {
    let instruction = build_initialize_ix(
        program_id,
        payer.pubkey(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        pyth_fee_lamports,
    );
    submit_tx(banks_client, payer, &[instruction], &[]).await;

    let (config_address, _) = config_pda(&program_id);
    let config_account = banks_client
        .get_account(config_address)
        .await
        .unwrap()
        .unwrap();
    let config = try_from_bytes::<Config>(&config_account.data).unwrap();
    assert_eq!(config.discriminator, config_discriminator());
}

pub async fn submit_tx(
    banks_client: &mut BanksClient,
    payer: &Keypair,
    instructions: &[Instruction],
    additional_signers: &[&Keypair],
) {
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let mut signers = Vec::with_capacity(1 + additional_signers.len());
    signers.push(payer);
    for signer in additional_signers {
        signers.push(*signer);
    }
    let mut transaction = Transaction::new_with_payer(instructions, Some(&payer.pubkey()));
    transaction.sign(&signers, recent_blockhash);
    banks_client.process_transaction(transaction).await.unwrap();
}
