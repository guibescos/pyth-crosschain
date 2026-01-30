use std::{path::PathBuf, str::FromStr};

use anyhow::{Context, Result};
use bytemuck::{bytes_of, cast_slice, try_from_bytes, Pod, Zeroable};
use clap::{Args, Parser, Subcommand, ValueEnum};
use entropy::{
    accounts::{CallbackMeta, Provider},
    constants::REQUESTER_SIGNER_SEED,
    instruction::EntropyInstruction,
    pda::{config_pda, provider_vault_pda, pyth_fee_vault_pda},
};
use simple_requester::{CALLBACK_ACTION, CALLBACK_STATE_LEN, REQUEST_WITH_CALLBACK_ACTION};
use solana_client::{rpc_client::RpcClient, rpc_config::RpcSendTransactionConfig};
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair, Signer},
    system_instruction, system_program,
    transaction::Transaction,
};

const DEFAULT_CALLBACK_COMPUTE_UNITS: u32 = 200_000;

#[derive(Parser, Debug)]
#[command(name = "entropy", about = "Entropy CLI tool", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Register a provider and listen for requests.
    Provide(ProvideArgs),
    /// Send a request to a provider.
    Request(RequestArgs),
}

#[derive(Args, Clone, Debug)]
struct SharedArgs {
    /// Solana RPC URL.
    #[arg(long, env = "SOLANA_RPC_URL", default_value = "http://localhost:8899")]
    rpc_url: String,

    /// Keypair file path.
    #[arg(long, env = "SOLANA_KEYPAIR", default_value = "~/.config/solana/id.json")]
    keypair: String,

    /// Commitment level.
    #[arg(long, value_enum, default_value_t = CommitmentArg::Confirmed)]
    commitment: CommitmentArg,
}

#[derive(Args, Debug)]
struct ProvideArgs {
    #[command(flatten)]
    shared: SharedArgs,

    /// Entropy program id.
    #[arg(long, value_name = "PROGRAM_ID")]
    entropy_program_id: Option<String>,
}

#[derive(Args, Debug)]
struct RequestArgs {
    #[command(flatten)]
    shared: SharedArgs,

    /// Provider id.
    #[arg(long, value_name = "PROVIDER_ID")]
    provider_id: String,

    /// Entropy program id.
    #[arg(long, env = "ENTROPY_PROGRAM_ID", value_name = "PROGRAM_ID")]
    entropy_program_id: Option<String>,

    /// Simple requester program id.
    #[arg(long, env = "SIMPLE_REQUESTER_PROGRAM_ID", value_name = "PROGRAM_ID")]
    requester_program_id: Option<String>,
}

#[derive(ValueEnum, Clone, Debug)]
enum CommitmentArg {
    Processed,
    Confirmed,
    Finalized,
}

impl CommitmentArg {
    fn to_config(&self) -> CommitmentConfig {
        let level = match self {
            CommitmentArg::Processed => CommitmentLevel::Processed,
            CommitmentArg::Confirmed => CommitmentLevel::Confirmed,
            CommitmentArg::Finalized => CommitmentLevel::Finalized,
        };
        CommitmentConfig { commitment: level }
    }
}

fn expand_path(path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::tilde(path);
    Ok(PathBuf::from(expanded.as_ref()))
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RequestWithCallbackHeader {
    user_randomness: [u8; 32],
    compute_unit_limit: u32,
    callback_accounts_len: u32,
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

fn parse_pubkey(value: &str, label: &str) -> Result<Pubkey> {
    Pubkey::from_str(value).with_context(|| format!("Invalid {label}: {value}"))
}

fn handle_provide(args: ProvideArgs) -> Result<()> {
    let keypair_path = expand_path(&args.shared.keypair)
        .with_context(|| format!("Invalid keypair path: {}", args.shared.keypair))?;
    let commitment = args.shared.commitment.to_config();

    println!("TODO: provide");
    println!("rpc url: {}", args.shared.rpc_url);
    println!("keypair: {}", keypair_path.display());
    println!("commitment: {:?}", commitment.commitment);
    println!("program id: {:?}", args.entropy_program_id);

    Ok(())
}

fn handle_request(args: RequestArgs) -> Result<()> {
    let keypair_path = expand_path(&args.shared.keypair)
        .with_context(|| format!("Invalid keypair path: {}", args.shared.keypair))?;
    let commitment = args.shared.commitment.to_config();

    let entropy_program_id = args
        .entropy_program_id
        .as_deref()
        .context("Missing --entropy-program-id (or ENTROPY_PROGRAM_ID)")?;
    let requester_program_id = args
        .requester_program_id
        .as_deref()
        .context("Missing --requester-program-id (or SIMPLE_REQUESTER_PROGRAM_ID)")?;

    let entropy_program_id = parse_pubkey(entropy_program_id, "entropy program id")?;
    let requester_program_id = parse_pubkey(requester_program_id, "requester program id")?;
    let provider_id = parse_pubkey(&args.provider_id, "provider id")?;

    let payer = read_keypair_file(&keypair_path)
        .with_context(|| format!("Failed to read keypair: {}", keypair_path.display()))?;
    let rpc_client =
        RpcClient::new_with_commitment(args.shared.rpc_url.clone(), commitment.clone());

    let provider_account = rpc_client
        .get_account(&provider_id)
        .with_context(|| format!("Failed to fetch provider account {provider_id}"))?;
    if provider_account.owner != entropy_program_id {
        return Err(anyhow::anyhow!(
            "Provider account owner mismatch: expected {}, got {}",
            entropy_program_id,
            provider_account.owner
        ));
    }
    let provider_data: Provider = *try_from_bytes(&provider_account.data)
        .with_context(|| "Invalid provider account data")?;
    let provider_authority = Pubkey::new_from_array(provider_data.provider_authority);

    let (provider_vault, _) = provider_vault_pda(&entropy_program_id, &provider_authority);
    let (config_account, _) = config_pda(&entropy_program_id);
    let (pyth_fee_vault, _) = pyth_fee_vault_pda(&entropy_program_id);
    let (requester_signer, _) = Pubkey::find_program_address(
        &[REQUESTER_SIGNER_SEED, entropy_program_id.as_ref()],
        &requester_program_id,
    );

    let request_account = Keypair::new();
    let callback_state = Keypair::new();

    let callback_state_rent = rpc_client
        .get_minimum_balance_for_rent_exemption(CALLBACK_STATE_LEN)
        .context("Failed to fetch rent exemption for callback state")?;
    let create_callback_state_ix = system_instruction::create_account(
        &payer.pubkey(),
        &callback_state.pubkey(),
        callback_state_rent,
        CALLBACK_STATE_LEN as u64,
        &requester_program_id,
    );

    let compute_unit_limit = if provider_data.default_compute_unit_limit > 0 {
        provider_data.default_compute_unit_limit
    } else {
        DEFAULT_CALLBACK_COMPUTE_UNITS
    };

    let user_randomness = Hash::new_unique().to_bytes();
    let callback_accounts = [CallbackMeta {
        pubkey: callback_state.pubkey().to_bytes(),
        is_signer: 0,
        is_writable: 1,
    }];

    let mut callback_ix_data = Vec::with_capacity(1 + 32);
    callback_ix_data.push(CALLBACK_ACTION);
    callback_ix_data.extend_from_slice(entropy_program_id.as_ref());

    let entropy_request_data = build_request_with_callback_data(
        user_randomness,
        compute_unit_limit,
        &callback_accounts,
        &callback_ix_data,
    );

    let mut requester_data = Vec::with_capacity(1 + entropy_request_data.len());
    requester_data.push(REQUEST_WITH_CALLBACK_ACTION);
    requester_data.extend_from_slice(&entropy_request_data);

    let request_with_callback_ix = Instruction {
        program_id: requester_program_id,
        data: requester_data,
        accounts: vec![
            AccountMeta::new_readonly(requester_signer, false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(requester_program_id, false),
            AccountMeta::new(request_account.pubkey(), true),
            AccountMeta::new(provider_id, false),
            AccountMeta::new(provider_vault, false),
            AccountMeta::new_readonly(config_account, false),
            AccountMeta::new(pyth_fee_vault, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(requester_program_id, false),
            AccountMeta::new_readonly(entropy_program_id, false),
        ],
    };

    let mut transaction = Transaction::new_with_payer(
        &[create_callback_state_ix, request_with_callback_ix],
        Some(&payer.pubkey()),
    );
    let blockhash = rpc_client.get_latest_blockhash()?;
    transaction.sign(&[&payer, &callback_state, &request_account], blockhash);

    let signature = rpc_client
        .send_and_confirm_transaction_with_spinner_and_config(
            &transaction,
            commitment,
            RpcSendTransactionConfig {
                skip_preflight: false,
                ..Default::default()
            },
        )
        .context("Request transaction failed")?;

    println!("request signature: {signature}");
    println!("request account: {}", request_account.pubkey());
    println!("callback state: {}", callback_state.pubkey());
    println!("requester signer: {requester_signer}");
    println!("provider vault: {provider_vault}");
    println!("config: {config_account}");
    println!("pyth fee vault: {pyth_fee_vault}");

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Provide(args) => handle_provide(args),
        Command::Request(args) => handle_request(args),
    }
}
