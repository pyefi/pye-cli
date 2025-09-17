use anyhow::Result;
use clap::{Parser, Subcommand};
use commands::transfer_excess_rewards::*;
use commands::validator_pye_account_manager::*;

pub mod accounts;
pub mod active_stake;
pub mod commands;
pub mod metrics_helpers;
pub mod rewards;
pub mod rpc_utils;
pub mod transactions;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Transfer excess rewards collected for the last completed epoch to SoloValiatorPyeAccount.
    TransferExcessRewards {
        /// RPC Endpoint
        #[arg(
            short,
            long,
            env,
            default_value = "https://api.mainnet-beta.solana.com"
        )]
        rpc: String,
        /// Path to payer keypair
        #[arg(short, long, env)]
        payer: String,
        /// SoloValidatorPyeAccount's pubkey
        #[arg(short, long, env)]
        pye_account: String,
        /// Maximum RPC requests to send concurrently.
        #[arg(long, env, default_value = "50")]
        concurrency: usize,
        /// Dry mode to calculate excess rewards without transferring.
        #[arg(long, env)]
        dry_run: bool,
        /// The wait time (in secs) between get_block RPC call retries.
        #[arg(long, env, default_value = "1800")]
        block_retry_delay: u64,
    },

    /// Will run the excess rewards stuff for all pye_accounts owned by a validator
    ValidatorPyeAccountManager {
        #[command(flatten)]
        args: ValidatorPyeAccountManagerArgs,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Setup logging to InfluxDB with solana_metrics
    env_logger::init();
    solana_metrics::set_host_id("pye_cli".to_string());
    solana_metrics::set_panic_hook("pye_cli", Some(env!("CARGO_PKG_VERSION").to_string()));

    match cli.command {
        Commands::TransferExcessRewards {
            rpc,
            payer,
            pye_account,
            concurrency,
            dry_run,
            block_retry_delay,
        } => {
            handle_transfer_excess_rewards(TransferExcessRewardsArgs {
                rpc,
                payer_file_path: payer,
                pye_account,
                concurrency,
                dry_run,
                block_retry_delay,
            })
            .await
        }
        Commands::ValidatorPyeAccountManager { args } => handle_validator_pye_account_manager(args).await,
    }
}
