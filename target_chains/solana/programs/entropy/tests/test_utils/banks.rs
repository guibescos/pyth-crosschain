use {
    bytemuck::try_from_bytes,
    entropy::{accounts::Config, discriminator::config_discriminator, pda::config_pda},
    solana_program_test::BanksClient,
    solana_sdk::{
        instruction::Instruction,
        signature::{Keypair, Signer},
        transaction::Transaction,
    },
};

use super::instructions::build_initialize_ix;

pub async fn initialize_config(
    banks_client: &mut BanksClient,
    payer: &Keypair,
    program_id: solana_program::pubkey::Pubkey,
    pyth_fee_lamports: u64,
) {
    let instruction = build_initialize_ix(
        program_id,
        payer.pubkey(),
        solana_program::pubkey::Pubkey::new_unique(),
        solana_program::pubkey::Pubkey::new_unique(),
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
