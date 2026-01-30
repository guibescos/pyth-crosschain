use std::{
    collections::HashSet,
    path::PathBuf,
    str::FromStr,
    thread::sleep,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use bs58::decode as bs58_decode;
use bytemuck::{bytes_of, try_from_bytes};
use clap::{Args, Parser, Subcommand, ValueEnum};
use entropy::{
    accounts::{CallbackMeta, Provider, Request},
    constants::{CALLBACK_NOT_STARTED, COMMITMENT_METADATA_LEN, URI_LEN},
    instruction::{EntropyInstruction, InitializeArgs, RegisterProviderArgs, RevealArgs},
    pda::{config_pda, entropy_signer_pda, provider_pda, provider_vault_pda, pyth_fee_vault_pda},
};
use rand::{rngs::OsRng, RngCore};
use solana_client::{
    rpc_client::{GetConfirmedSignaturesForAddress2Config, RpcClient},
    rpc_config::{RpcSendTransactionConfig, RpcTransactionConfig},
};
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_sdk::{
    hash::hash,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair, Signature},
    signer::Signer,
    system_program, sysvar::slot_hashes,
    transaction::Transaction,
};
use solana_transaction_status::{
    option_serializer::OptionSerializer, EncodedConfirmedTransactionWithStatusMeta,
    EncodedTransaction, UiCompiledInstruction, UiInstruction, UiMessage, UiTransaction,
    UiTransactionEncoding,
};
use bytemuck::{cast_slice, Pod, Zeroable};
use entropy::{
    constants::REQUESTER_SIGNER_SEED,
};
use simple_requester::{CALLBACK_ACTION, CALLBACK_STATE_LEN, REQUEST_WITH_CALLBACK_ACTION};
use solana_sdk::{
    hash::Hash,
    system_instruction,
};

const DEFAULT_CALLBACK_COMPUTE_UNITS: u32 = 200_000;

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

struct ProviderChain {
    chain: Vec<[u8; 32]>,
    current_index: usize,
    current_sequence: u64,
}

#[derive(Clone, Debug)]
struct RequestObservation {
    request_account: Pubkey,
    provider_account: Pubkey,
    user_randomness: [u8; 32],
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

fn load_keypair(path: &PathBuf) -> Result<Keypair> {
    read_keypair_file(path).map_err(|err| {
        anyhow::anyhow!("Failed to read keypair file {}: {err}", path.display())
    })
}

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
    args: RegisterProviderArgs,
) -> Instruction {
    let (provider_account, _) = provider_pda(&program_id, &provider_authority);
    let (provider_vault, _) = provider_vault_pda(&program_id, &provider_authority);

    let mut data = Vec::with_capacity(8 + core::mem::size_of::<RegisterProviderArgs>());
    data.extend_from_slice(&EntropyInstruction::RegisterProvider.discriminator());
    data.extend_from_slice(bytes_of(&args));

    Instruction {
        program_id,
        data,
        accounts: vec![
            AccountMeta::new(provider_authority, true),
            AccountMeta::new(provider_account, false),
            AccountMeta::new(provider_vault, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
    }
}

fn build_reveal_with_callback_ix(
    program_id: Pubkey,
    request_account: Pubkey,
    provider_account: Pubkey,
    entropy_signer: Pubkey,
    callback_program: Pubkey,
    payer: Pubkey,
    callback_accounts: &[CallbackMeta],
    args: RevealArgs,
) -> Instruction {
    let mut data = Vec::with_capacity(8 + core::mem::size_of::<RevealArgs>());
    data.extend_from_slice(&EntropyInstruction::RevealWithCallback.discriminator());
    data.extend_from_slice(bytes_of(&args));

    let mut accounts = Vec::with_capacity(7 + callback_accounts.len());
    accounts.push(AccountMeta::new(request_account, false));
    accounts.push(AccountMeta::new(provider_account, false));
    accounts.push(AccountMeta::new_readonly(slot_hashes::id(), false));
    accounts.push(AccountMeta::new_readonly(entropy_signer, false));
    accounts.push(AccountMeta::new_readonly(callback_program, false));
    accounts.push(AccountMeta::new_readonly(system_program::id(), false));
    accounts.push(AccountMeta::new(payer, false));

    for meta in callback_accounts {
        let key = Pubkey::new_from_array(meta.pubkey);
        if meta.is_writable == 1 {
            accounts.push(AccountMeta::new(key, meta.is_signer == 1));
        } else {
            accounts.push(AccountMeta::new_readonly(key, meta.is_signer == 1));
        }
    }

    Instruction {
        program_id,
        data,
        accounts,
    }
}

fn send_and_confirm(
    rpc_client: &RpcClient,
    payer: &Keypair,
    instructions: &[Instruction],
    commitment: CommitmentConfig,
    label: &str,
) -> Result<Signature> {
    let mut attempt = 0;
    let mut backoff = Duration::from_millis(500);
    loop {
        attempt += 1;
        let recent_blockhash = rpc_client.get_latest_blockhash()?;
        let mut transaction = Transaction::new_with_payer(instructions, Some(&payer.pubkey()));
        transaction.sign(&[payer], recent_blockhash);

        let sig = rpc_client.send_transaction_with_config(
            &transaction,
            RpcSendTransactionConfig {
                skip_preflight: true,
                preflight_commitment: Some(commitment.commitment),
                ..RpcSendTransactionConfig::default()
            },
        );

        sleep(Duration::from_secs(2));
        match sig {
            Ok(signature) => {
                let confirmed = rpc_client
                    .confirm_transaction_with_commitment(&signature, commitment)?
                    .value;
                if confirmed {
                    return Ok(signature);
                }
            }
            Err(err) => {
                eprintln!("{} attempt {} failed: {err}", label, attempt);
            }
        }

        if attempt >= 6 {
            bail!("{} failed after {} attempts", label, attempt);
        }
        sleep(backoff);
        backoff = std::cmp::min(backoff * 2, Duration::from_secs(8));
    }
}

fn build_register_args(commitment: [u8; 32], chain_length: u64) -> RegisterProviderArgs {
    let commitment_metadata = [0u8; COMMITMENT_METADATA_LEN];
    let uri = [0u8; URI_LEN];

    RegisterProviderArgs {
        fee_lamports: 0,
        commitment,
        commitment_metadata_len: 0,
        _padding0: [0u8; 6],
        commitment_metadata,
        chain_length,
        uri_len: 0,
        uri,
        _padding1: [0u8; 6],
    }
}

fn build_chain(chain_length: usize) -> ([u8; 32], Vec<[u8; 32]>) {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let mut chain = Vec::with_capacity(chain_length + 1);
    chain.push(seed);
    for index in 0..chain_length {
        let next = hash(&chain[index]).to_bytes();
        chain.push(next);
    }
    let commitment = *chain.last().expect("chain is non-empty");
    (commitment, chain)
}

fn parse_user_randomness(data: &[u8]) -> Option<[u8; 32]> {
    let discriminator = EntropyInstruction::RequestWithCallback.discriminator();
    if data.len() < 8 + 32 || data[..8] != discriminator {
        return None;
    }
    let mut user_randomness = [0u8; 32];
    user_randomness.copy_from_slice(&data[8..40]);
    Some(user_randomness)
}

fn parse_request_observations(
    tx: &EncodedConfirmedTransactionWithStatusMeta,
    entropy_program_id: &Pubkey,
) -> Result<Vec<RequestObservation>> {
    let message = match &tx.transaction.transaction {
        EncodedTransaction::Json(UiTransaction { message, .. }) => message,
        _ => return Ok(Vec::new()),
    };

    let raw_message = match message {
        UiMessage::Raw(raw) => raw,
        _ => return Ok(Vec::new()),
    };

    let account_keys: Vec<Pubkey> = raw_message
        .account_keys
        .iter()
        .map(|key| Pubkey::from_str(key))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to parse account keys")?;

    let mut observations = Vec::new();

    for instruction in &raw_message.instructions {
        collect_compiled_observation(
            instruction,
            &account_keys,
            entropy_program_id,
            &mut observations,
        );
    }

    if let Some(meta) = &tx.transaction.meta {
        if let OptionSerializer::Some(inner) = &meta.inner_instructions {
            for inner_ix in inner {
                for instruction in &inner_ix.instructions {
                    collect_request_observation(
                        instruction,
                        &account_keys,
                        entropy_program_id,
                        &mut observations,
                    );
                }
            }
        }
    }

    Ok(observations)
}

fn collect_request_observation(
    instruction: &UiInstruction,
    account_keys: &[Pubkey],
    entropy_program_id: &Pubkey,
    observations: &mut Vec<RequestObservation>,
) {
    let compiled = match instruction {
        UiInstruction::Compiled(compiled) => compiled,
        _ => return,
    };

    let program_key = match account_keys.get(compiled.program_id_index as usize) {
        Some(key) => key,
        None => return,
    };
    if program_key != entropy_program_id {
        return;
    }

    let data = match bs58_decode(&compiled.data).into_vec() {
        Ok(data) => data,
        Err(_) => return,
    };
    let user_randomness = match parse_user_randomness(&data) {
        Some(randomness) => randomness,
        None => return,
    };

    let request_index = match compiled.accounts.get(3) {
        Some(index) => *index as usize,
        None => return,
    };
    let provider_index = match compiled.accounts.get(4) {
        Some(index) => *index as usize,
        None => return,
    };

    let request_account = match account_keys.get(request_index) {
        Some(key) => *key,
        None => return,
    };
    let provider_account = match account_keys.get(provider_index) {
        Some(key) => *key,
        None => return,
    };

    observations.push(RequestObservation {
        request_account,
        provider_account,
        user_randomness,
    });
}

fn collect_compiled_observation(
    instruction: &UiCompiledInstruction,
    account_keys: &[Pubkey],
    entropy_program_id: &Pubkey,
    observations: &mut Vec<RequestObservation>,
) {
    let program_key = match account_keys.get(instruction.program_id_index as usize) {
        Some(key) => key,
        None => return,
    };
    if program_key != entropy_program_id {
        return;
    }

    let data = match bs58_decode(&instruction.data).into_vec() {
        Ok(data) => data,
        Err(_) => return,
    };
    let user_randomness = match parse_user_randomness(&data) {
        Some(randomness) => randomness,
        None => return,
    };

    let request_index = match instruction.accounts.get(3) {
        Some(index) => *index as usize,
        None => return,
    };
    let provider_index = match instruction.accounts.get(4) {
        Some(index) => *index as usize,
        None => return,
    };

    let request_account = match account_keys.get(request_index) {
        Some(key) => *key,
        None => return,
    };
    let provider_account = match account_keys.get(provider_index) {
        Some(key) => *key,
        None => return,
    };

    observations.push(RequestObservation {
        request_account,
        provider_account,
        user_randomness,
    });
}

fn handle_provide(args: ProvideArgs) -> Result<()> {
    let keypair_path = expand_path(&args.shared.keypair)
        .with_context(|| format!("Invalid keypair path: {}", args.shared.keypair))?;
    let commitment = args.shared.commitment.to_config();
    let rpc_client = RpcClient::new_with_commitment(args.shared.rpc_url.clone(), commitment);
    let payer = load_keypair(&keypair_path)?;

    let entropy_program_id = args
        .entropy_program_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--entropy-program-id is required"))?;
    let entropy_program_id = Pubkey::from_str(entropy_program_id)
        .with_context(|| format!("Invalid entropy program id: {}", entropy_program_id))?;

    println!("rpc url: {}", args.shared.rpc_url);
    println!("keypair: {}", keypair_path.display());
    println!("commitment: {:?}", commitment.commitment);
    println!("program id: {}", entropy_program_id);

    let (config_address, _) = config_pda(&entropy_program_id);
    if rpc_client.get_account(&config_address).is_err() {
        println!("Initializing entropy config...");
        let ix = build_initialize_ix(
            entropy_program_id,
            payer.pubkey(),
            payer.pubkey(),
            payer.pubkey(),
            0,
        );
        send_and_confirm(&rpc_client, &payer, &[ix], commitment, "initialize")?;
    } else {
        println!("Entropy config already initialized.");
    }

    let chain_length = 256u64;
    let (commitment_value, chain) = build_chain(chain_length as usize);
    let register_args = build_register_args(commitment_value, chain_length);
    let register_ix = build_register_provider_ix(entropy_program_id, payer.pubkey(), register_args);
    println!("Registering provider...");
    send_and_confirm(
        &rpc_client,
        &payer,
        &[register_ix],
        commitment,
        "register provider",
    )?;

    let (provider_account, _) = provider_pda(&entropy_program_id, &payer.pubkey());
    let provider_data = rpc_client
        .get_account_data(&provider_account)
        .context("Failed to fetch provider account")?;
    let provider = try_from_bytes::<Provider>(&provider_data)
        .map_err(|err| anyhow::anyhow!("Failed to parse provider account: {err}"))?;

    let mut provider_chain = ProviderChain {
        chain,
        current_index: chain_length as usize,
        current_sequence: provider.current_commitment_sequence_number,
    };

    println!("Provider authority: {}", payer.pubkey());
    println!("Provider account: {}", provider_account);
    println!("Listening for request_with_callback...");

    let mut processed_signatures = HashSet::new();
    let mut last_seen: Option<String> = None;
    loop {
        let signatures = rpc_client.get_signatures_for_address_with_config(
            &entropy_program_id,
            GetConfirmedSignaturesForAddress2Config {
                limit: Some(100),
                ..GetConfirmedSignaturesForAddress2Config::default()
            },
        );

        let signatures = match signatures {
            Ok(sigs) => sigs,
            Err(err) => {
                eprintln!("Failed to fetch signatures: {err}");
                sleep(Duration::from_secs(2));
                continue;
            }
        };

        if signatures.is_empty() {
            sleep(Duration::from_secs(2));
            continue;
        }

        let mut new_signatures = Vec::new();
        for sig in &signatures {
            if last_seen.as_deref() == Some(&sig.signature) {
                break;
            }
            if processed_signatures.insert(sig.signature.clone()) {
                new_signatures.push(sig.signature.clone());
            }
        }

        if let Some(first) = signatures.first() {
            last_seen = Some(first.signature.clone());
        }

        new_signatures.reverse();

        for signature_str in new_signatures {
            let signature = match Signature::from_str(&signature_str) {
                Ok(sig) => sig,
                Err(_) => continue,
            };
            let tx = rpc_client.get_transaction_with_config(
                &signature,
                RpcTransactionConfig {
                    encoding: Some(UiTransactionEncoding::Json),
                    commitment: Some(commitment),
                    max_supported_transaction_version: Some(0),
                },
            );
            let tx = match tx {
                Ok(tx) => tx,
                Err(err) => {
                    eprintln!("Failed to fetch transaction {signature_str}: {err}");
                    continue;
                }
            };

            let observations = parse_request_observations(&tx, &entropy_program_id)?;
            for observation in observations {
                if observation.provider_account != provider_account {
                    continue;
                }

                let request_data = match rpc_client.get_account_data(&observation.request_account)
                {
                    Ok(data) => data,
                    Err(err) => {
                        eprintln!(
                            "Failed to fetch request {}: {err}",
                            observation.request_account
                        );
                        continue;
                    }
                };
                let request = match try_from_bytes::<Request>(&request_data) {
                    Ok(request) => request,
                    Err(err) => {
                        eprintln!(
                            "Failed to parse request {}: {err}",
                            observation.request_account
                        );
                        continue;
                    }
                };

                if request.callback_status != CALLBACK_NOT_STARTED {
                    continue;
                }
                if Pubkey::new_from_array(request.provider) != payer.pubkey() {
                    continue;
                }
                if request.sequence_number <= provider_chain.current_sequence {
                    continue;
                }

                let num_hashes = request
                    .sequence_number
                    .checked_sub(provider_chain.current_sequence)
                    .unwrap_or(0);
                let num_hashes_usize = match usize::try_from(num_hashes) {
                    Ok(value) => value,
                    Err(_) => {
                        eprintln!("Sequence number too large: {}", request.sequence_number);
                        continue;
                    }
                };
                if num_hashes_usize > provider_chain.current_index {
                    eprintln!("Out of provider randomness. Re-register provider.");
                    continue;
                }

                let provider_contribution =
                    provider_chain.chain[provider_chain.current_index - num_hashes_usize];
                let reveal_args = RevealArgs {
                    user_contribution: observation.user_randomness,
                    provider_contribution,
                };

                let entropy_signer = entropy_signer_pda(&entropy_program_id).0;
                let callback_program = Pubkey::new_from_array(request.requester_program_id);

                let callback_accounts_len = request.callback_accounts_len as usize;
                let callback_accounts = &request.callback_accounts[..callback_accounts_len];

                let reveal_ix = build_reveal_with_callback_ix(
                    entropy_program_id,
                    observation.request_account,
                    provider_account,
                    entropy_signer,
                    callback_program,
                    Pubkey::new_from_array(request.payer),
                    callback_accounts,
                    reveal_args,
                );

                println!(
                    "Revealing for request {} (sequence {})",
                    observation.request_account, request.sequence_number
                );

                match send_and_confirm(
                    &rpc_client,
                    &payer,
                    &[reveal_ix],
                    commitment,
                    "reveal_with_callback",
                ) {
                    Ok(_) => {
                        provider_chain.current_index -= num_hashes_usize;
                        provider_chain.current_sequence = request.sequence_number;
                    }
                    Err(err) => {
                        eprintln!("Failed to reveal: {err}");
                    }
                }
            }
        }

        sleep(Duration::from_secs(2));
    }

    #[allow(unreachable_code)]
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

    let payer = read_keypair_file(&keypair_path).unwrap();
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
        .unwrap();
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
                preflight_commitment: Some(commitment.commitment),
                ..RpcSendTransactionConfig::default()
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
