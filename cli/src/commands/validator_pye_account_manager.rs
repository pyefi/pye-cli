use std::{sync::Arc, time::Duration};

use anchor_client::Cluster;
use anyhow::{anyhow, Result};
use clap::Parser;
use futures::stream::{self, StreamExt};
use log::info;
use pye_core_cpi::pye_core::accounts::SoloValidatorBond as SoloValidatorPyeAccount;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_metrics::{datapoint_error, datapoint_info, flush};
use solana_sdk::pubkey::Pubkey;

use crate::{
    accounts::fetch_active_solo_validator_pye_accounts_by_vote_key_and_issuer,
    active_stake::fetch_pye_account_active_stake,
    metrics_helpers::{log_reward_commissions, log_validator_mev_data},
    rewards::{
        block_rewards::{calculate_block_rewards, compute_excess_block_commission},
        inflation_rewards::calculate_excess_inflation_reward,
        mev_rewards::{calculate_excess_mev_reward, fetch_and_filter_mev_data},
    },
    rpc_utils::wait_for_next_epoch,
    transactions::transfer_excess_rewards,
};

#[derive(Clone, Debug, Parser)]
pub struct ValidatorPyeAccountManagerArgs {
    /// RPC Endpoint
    #[arg(
        short,
        long,
        env,
        default_value = "https://api.mainnet-beta.solana.com"
    )]
    rpc: String,
    /// The Pye program ID
    #[arg(
        long,
        env,
        default_value = "PYEQZ2qYHPQapnw8Ms8MSPMNzoq59NHHfNwAtuV26wx"
    )]
    program_id: Pubkey,
    /// Validator's vote accoutn
    #[arg(short, long, env)]
    vote_pubkey: Pubkey,
    /// Restricts pye_account payments to only pye_accounts issued by pubkeys in this list.
    #[arg(short, long, env, value_delimiter = ',')]
    issuers: Vec<Pubkey>,
    /// Path to payer keypair
    #[arg(short, long, env)]
    payer: String,
    /// Maximum RPC requests to send concurrently.
    #[arg(long, env, default_value = "50")]
    concurrency: usize,
    /// Dry mode to calculate excess rewards without transferring.
    #[arg(long, env)]
    dry_run: bool,
    /// The wait time (in secs) between epoch change checks
    #[arg(long, env, default_value = "60")]
    cycle_secs: u64,
    /// The wait time (in secs) between get_block RPC call retries.
    #[arg(long, env, default_value = "1800")]
    block_retry_delay: u64,
}

pub async fn handle_validator_pye_account_manager(args: ValidatorPyeAccountManagerArgs) -> Result<()> {
    let rpc_client = Arc::new(RpcClient::new_with_commitment(
        args.rpc.clone(),
        CommitmentConfig::confirmed(),
    ));

    let epoch_schedule = rpc_client.get_epoch_schedule().await?;
    let mut current_epoch_info = match rpc_client.get_epoch_info().await {
        Ok(info) => info,
        Err(err) => {
            datapoint_error!(
                "handle_validator_pye_account_manager",
                ("error", err.to_string(), String),
            );
            return Err(anyhow!("Error getting epoch info: {:?}", err));
        }
    };
    loop {
        // Fetch pye_accounts that are still active prior to waiting for the next epoch, to make sure we
        // don't miss any.
        let results: Vec<_> = stream::iter(args.issuers.to_owned())
            .map(|issuer_pubkey| {
                let cloned_client = rpc_client.clone();
                async move {
                    match fetch_active_solo_validator_pye_accounts_by_vote_key_and_issuer(
                        &cloned_client,
                        &args.program_id,
                        &args.vote_pubkey,
                        &issuer_pubkey.clone(),
                    )
                    .await
                    {
                        Ok(pye_accounts) => Ok(pye_accounts),
                        Err(err) => {
                            datapoint_error!(
                                "handle_validator_pye_account_manager",
                                ("error", err.to_string(), String),
                            );
                            return Err(anyhow!("Error fetching active pye_accounts: {:?}", err));
                        }
                    }
                }
            })
            .buffer_unordered(args.concurrency)
            .collect::<Vec<_>>()
            .await;
        let active_pye_accounts: Vec<(Pubkey, SoloValidatorPyeAccount)> = results
            .into_iter()
            .filter_map(Result::ok)
            .flatten()
            .collect();

        info!(
            "Monitoring {} pye_accounts for epoch {}",
            active_pye_accounts.len(),
            current_epoch_info.epoch
        );
        // We block the flow until the next epoch
        current_epoch_info =
            wait_for_next_epoch(&rpc_client, current_epoch_info.epoch, args.cycle_secs).await;
        // We wait 30 seconds to avoid "Epoch rewards period still active at slot" RPC errors
        tokio::time::sleep(Duration::from_secs(30)).await;
        info!(
            "Epoch boundary detected. New epoch: {}",
            current_epoch_info.epoch
        );
        let target_epoch = current_epoch_info.epoch - 1;
        let last_slot_of_target = epoch_schedule.get_last_slot_in_epoch(target_epoch);

        let block_time = match rpc_client.get_block_time(last_slot_of_target).await {
            Err(_) => {
                // TODO: Get a more accurate time of the end of the epoch to determine if payment
                // should be made. One idea is catch RpcError::ForUser and check for next block
                // in subsequent slots
                let now = chrono::Utc::now();
                now.timestamp()
            }
            Ok(block_time) => block_time,
        };

        // For all active pye_accounts, log their commission structures and filter by maturity
        let active_pye_accounts: Vec<(Pubkey, SoloValidatorPyeAccount)> = active_pye_accounts
            .into_iter()
            .filter(|(pye_account_pubkey, pye_account)| {
                log_reward_commissions(target_epoch, &pye_account_pubkey, &pye_account.reward_commissions);
                pye_account.maturity_ts > block_time
            })
            .collect();

        // Load MEV data
        let mev_data = fetch_and_filter_mev_data(&args.vote_pubkey, target_epoch).await?;
        log_validator_mev_data(target_epoch, &mev_data);

        let validators_total_block_rewards = calculate_block_rewards(
            &rpc_client,
            &args.vote_pubkey,
            &current_epoch_info,
            args.concurrency,
            args.block_retry_delay,
        )
        .await?;

        // Note: could add concurrency in this loop
        // For each pye_account calculate the additional rewards required for each category
        for (pye_account_pubkey, pye_account) in active_pye_accounts.into_iter() {
            // Fetch the SoloValidatorPyeAccount's active stake during target epoch.
            let pye_account_active_stake = fetch_pye_account_active_stake(
                &rpc_client,
                &pye_account.stake_account,
                &pye_account.transient_stake_account,
                target_epoch,
                current_epoch_info.epoch,
            )
            .await?;
            // Calculate the excess inflation reward to be refunded by validator to SoloValidatorPyeAccount.
            let excess_inflation_reward = calculate_excess_inflation_reward(
                &rpc_client,
                &pye_account.stake_account,
                &pye_account.transient_stake_account,
                target_epoch,
                &pye_account.reward_commissions,
            )
            .await;

            // Calculate the excess MEV reward to be refunded by validator to SoloValidatorPyeAccount.
            let excess_mev_commission =
                calculate_excess_mev_reward(&mev_data, pye_account_active_stake, &pye_account.reward_commissions);

            // Calculate the excess block reward to be funded by validator to SoloValidatorPyeAccount.
            let excess_block_commission = compute_excess_block_commission(
                validators_total_block_rewards,
                pye_account_active_stake,
                mev_data.active_stake,
                pye_account.reward_commissions.block_rewards_bps,
            );

            let excess_rewards =
                excess_inflation_reward + excess_block_commission + excess_mev_commission;

            info!(
                "pye_account: {}\nSOL to transfer: {}\n\n",
                pye_account_pubkey, excess_rewards
            );

            datapoint_info!(
                "excess_reward",
                ("vote_pubkey", args.vote_pubkey.to_string(), String),
                ("epoch", target_epoch.to_string(), String),
                ("pye_account", pye_account_pubkey.to_string(), String),
                ("pye_account_active_stake", pye_account_active_stake as i64, i64),
                ("excess_inflation_rewards", excess_inflation_reward, i64),
                ("excess_mev_rewards", excess_mev_commission, i64),
                ("excess_block_rewards", excess_block_commission, i64),
                ("total_excess_rewards", excess_rewards, i64),
            );

            if excess_rewards <= 0 {
                info!(
                    "No excess rewards to transfer to pye_account {} for epoch {}\n",
                    pye_account_pubkey, target_epoch
                );
                continue;
            }

            // Make the actual SOL transfer if not a dry run and rewards are greater than 0
            if !args.dry_run {
                // transfer_excess_rewards_with_delegate_tips
                let cluster = Cluster::Custom(args.rpc.clone(), args.rpc.replace("http", "ws"));
                transfer_excess_rewards(
                    args.payer.clone(),
                    cluster,
                    &pye_account_pubkey,
                    &pye_account,
                    u64::try_from(excess_rewards)?,
                )
                .await
                .map_err(|e| anyhow!("Failed to transfer excess rewards: {}", e))?
            }
        }
        flush();
    }
}
