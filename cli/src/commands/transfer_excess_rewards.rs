use crate::accounts::fetch_solo_validator_pye_account;
use crate::active_stake::fetch_pye_account_active_stake;
use crate::metrics_helpers::*;
use crate::rewards::block_rewards::calculate_excess_block_reward;
use crate::rewards::inflation_rewards::calculate_excess_inflation_reward;
use crate::rewards::mev_rewards::{calculate_excess_mev_reward, fetch_and_filter_mev_data};
use crate::transactions::transfer_excess_rewards;
use anchor_client::Cluster;
use anyhow::{anyhow, Result};
use dialoguer::Confirm;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_metrics::{datapoint_info, flush};
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey};
use std::str::FromStr;

pub struct TransferExcessRewardsArgs {
    pub rpc: String,
    pub payer_file_path: String,
    pub pye_account: String,
    pub concurrency: usize,
    pub dry_run: bool,
    pub block_retry_delay: u64,
}

pub async fn handle_transfer_excess_rewards(args: TransferExcessRewardsArgs) -> Result<()> {
    let client = RpcClient::new_with_commitment(args.rpc.clone(), CommitmentConfig::confirmed());
    let pye_account_pubkey = Pubkey::from_str(&args.pye_account).map_err(|e| anyhow!("Invalid pye_account: {}", e))?;

    // Fetch RewardCommissions configured on SoloValidatorPyeAccount.
    let pye_account = fetch_solo_validator_pye_account(&client, &pye_account_pubkey).await?;
    let reward_commissions = pye_account.reward_commissions.clone();
    info!("Current: {:?}", reward_commissions);

    // Fetch the current Solana Network epoch.
    let epoch_info = client.get_epoch_info().await?;
    let current_epoch = epoch_info.epoch;
    let target_epoch = current_epoch - 1;
    println!("Current epoch: {}\n", current_epoch);
    log_reward_commissions(target_epoch, &pye_account_pubkey, &reward_commissions);

    // Fetch info about MEV rewards for target epoch from Jito's API.
    let mev_data = fetch_and_filter_mev_data(&pye_account.validator_vote_account, target_epoch).await?;
    log_validator_mev_data(target_epoch, &mev_data);

    // Fetch the SoloValidatorPyeAccount's active stake during target epoch.
    let pye_account_active_stake = fetch_pye_account_active_stake(
        &client,
        &pye_account.stake_account,
        &pye_account.transient_stake_account,
        target_epoch,
        current_epoch,
    )
    .await?;

    // Calculate the excess inflation reward to be refunded by validator to SoloValidatorPyeAccount.
    let excess_inflation_reward = calculate_excess_inflation_reward(
        &client,
        &pye_account.stake_account,
        &pye_account.transient_stake_account,
        target_epoch,
        &reward_commissions,
    )
    .await;

    // Calculate the excess MEV reward to be refunded by validator to SoloValidatorPyeAccount.
    let excess_mev_commission =
        calculate_excess_mev_reward(&mev_data, pye_account_active_stake, &reward_commissions);

    // Calculate the excess block reward to be refunded by validator to SoloValidatorPyeAccount.
    let excess_block_commission = calculate_excess_block_reward(
        &client,
        &pye_account.validator_vote_account,
        &epoch_info,
        pye_account_active_stake,
        mev_data.active_stake,
        &reward_commissions,
        args.concurrency,
        args.block_retry_delay,
    )
    .await?;

    let excess_rewards = excess_inflation_reward + excess_block_commission + excess_mev_commission;
    println!("Total Excess Rewards: {}\n", excess_rewards);

    datapoint_info!(
        "excess_reward",
        (
            "vote_pubkey",
            pye_account.validator_vote_account.to_string(),
            String
        ),
        ("epoch", target_epoch.to_string(), String),
        ("pye_account", pye_account_pubkey.to_string(), String),
        ("pye_account_active_stake", pye_account_active_stake as i64, i64),
        ("excess_inflation_rewards", excess_inflation_reward, i64),
        ("excess_mev_rewards", excess_mev_commission, i64),
        ("excess_block_rewards", excess_block_commission, i64),
        ("total_excess_rewards", excess_rewards, i64),
    );
    flush();

    if excess_rewards <= 0 {
        info!(
            "No excess rewards to transfer to SoloValidatorPyeAccount for epoch {}\n",
            target_epoch
        );
        return Ok(());
    }

    if args.dry_run {
        info!("Dry run complete");
        return Ok(());
    }

    if Confirm::new()
        .with_prompt(format!(
            "Transfer {} lamports in excess rewards to SoloValidatorPyeAccount at {}?",
            excess_rewards, pye_account_pubkey
        ))
        .interact()?
    {
        let cluster = Cluster::Custom(args.rpc.clone(), args.rpc.replace("http", "ws"));
        transfer_excess_rewards(
            args.payer_file_path,
            cluster,
            &pye_account_pubkey,
            &pye_account,
            u64::try_from(excess_rewards)?,
        )
        .await
        .map_err(|e| anyhow!("Failed to transfer excess rewards: {}", e))
    } else {
        info!("Aborted: user declined to transfer excess rewards.");
        Ok(())
    }
}
