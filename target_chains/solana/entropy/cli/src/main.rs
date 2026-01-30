use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};

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
    provider_id: Option<String>,
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

    println!("TODO: request");
    println!("rpc url: {}", args.shared.rpc_url);
    println!("keypair: {}", keypair_path.display());
    println!("commitment: {:?}", commitment.commitment);
    println!("provider id: {:?}", args.provider_id);

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Provide(args) => handle_provide(args),
        Command::Request(args) => handle_request(args),
    }
}
