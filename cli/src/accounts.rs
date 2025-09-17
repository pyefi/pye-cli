use anchor_lang::{AccountDeserialize, Discriminator};
use anyhow::{anyhow, Error};
use log::info;
use pye_core_cpi::pye_core::accounts::SoloValidatorBond as SoloValidatorPyeAccount;
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::{Memcmp, MemcmpEncodedBytes, RpcFilterType};
use solana_sdk::account::from_account;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::stake_history::StakeHistory;
use solana_sdk::sysvar::{slot_history, stake_history};

pub async fn fetch_stake_history(client: &RpcClient) -> Result<StakeHistory, Error> {
    let account_data = client
        .get_account(&stake_history::ID)
        .await
        .map_err(|e| anyhow!("Failed to fetch StakeHistory: {}", e))?;
    let stake_history: StakeHistory = from_account::<StakeHistory, _>(&account_data)
        .ok_or_else(|| anyhow!("Failed to deserialize StakeHistory"))?;
    Ok(stake_history)
}

pub async fn fetch_slot_history(client: &RpcClient) -> Result<slot_history::SlotHistory, Error> {
    let account_data = client
        .get_account(&slot_history::ID)
        .await
        .map_err(|e| anyhow!("Failed to fetch SlotHistory: {}", e))?;
    let slot_history = from_account::<slot_history::SlotHistory, _>(&account_data)
        .ok_or_else(|| anyhow!("Failed to deserialize SlotHistory"))?;
    Ok(slot_history)
}

pub async fn fetch_solo_validator_pye_account(
    client: &RpcClient,
    pye_account_pubkey: &Pubkey,
) -> Result<SoloValidatorPyeAccount, Error> {
    let account_data = client
        .get_account_data(&pye_account_pubkey)
        .await
        .map_err(|e| anyhow!("Failed to fetch SoloValidatorPyeAccount: {}", e))?;
    let pye_account = SoloValidatorPyeAccount::try_deserialize(&mut account_data.as_slice())
        .map_err(|e| anyhow!("Failed to deserialize SoloValidatorPyeAccount: {}", e))?;
    Ok(pye_account)
}

pub async fn fetch_active_solo_validator_pye_accounts_by_vote_key_and_issuer(
    client: &RpcClient,
    program_id: &Pubkey,
    vote_pubkey: &Pubkey,
    issuer_pubkey: &Pubkey,
) -> Result<Vec<(Pubkey, SoloValidatorPyeAccount)>, Error> {
    let discriminator_filter = RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
        0,
        SoloValidatorPyeAccount::DISCRIMINATOR,
    ));
    let vote_pubkey_filter = RpcFilterType::Memcmp(Memcmp::new(
        8,
        MemcmpEncodedBytes::Base58(vote_pubkey.to_string()),
    ));
    let issuer_pubkey_filter = RpcFilterType::Memcmp(Memcmp::new(
        240,
        MemcmpEncodedBytes::Base58(issuer_pubkey.to_string()),
    ));
    let not_matured_filter = RpcFilterType::Memcmp(Memcmp::new_base58_encoded(185, &[0]));
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![
            discriminator_filter,
            vote_pubkey_filter,
            not_matured_filter,
            issuer_pubkey_filter,
        ]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64Zstd),
            data_slice: None,
            commitment: None,
            min_context_slot: None,
        },
        with_context: None,
        sort_results: None,
    };
    let accounts = client
        .get_program_accounts_with_config(program_id, config)
        .await
        .map_err(|e| anyhow!("Failed to fetch SoloValidatorPyeAccount: {}", e))?;
    info!(
        "Fetched {} active pye-accounts for issuer {}",
        accounts.len(),
        issuer_pubkey
    );

    Ok(accounts
        .into_iter()
        .map(|(pubkey, account)| {
            let mut data: &[u8] = &account.data;
            let pye_account = SoloValidatorPyeAccount::try_deserialize(&mut data).unwrap();
            (pubkey, pye_account)
        })
        .collect())
}
