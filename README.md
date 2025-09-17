# PYE CLI

A lightweight command‑line interface for interacting with **Pye Accounts** program on Solana. It is intended for validator operators who need to transfer staking rewards to Pye Account holders and future applications.

---

## Building

```bash
# Clone and compile Pye
$ git clone https://github.com/pyefi/pye-program-library.git
$ cd pye-program-library/cli
$ cargo build --release
```

The optimised binary will be at `target/release/pye-cli`.

To run directly from source with hot‑reload:

```bash
$ cargo run -- <COMMAND> [OPTIONS]
```

---

## Commands

### `transfer-excess-rewards`

Transfer any excess MEV rewards from the last completed epoch into the Pye account.

**Usage:**

```sh
pye-cli transfer-excess-rewards \
  --rpc <RPC_URL> \
  --payer <KEYPAIR_PATH> \
  --pye-account <PYE_ACCOUNT_PUBKEY> \
  [--concurrency <NUMBER>] \
  [--dry-run] \
  [--block-retry-delay <BLOCK_RETRY_DELAY>]
```

**Example:**

```sh
./target/release/pye-cli transfer-excess-rewards \
  --rpc https://api.mainnet-beta.solana.com \
  --payer ~/.config/solana/id.json \
  --pye-account HETNBL5z4Q1xPw2kTpAR462TPRwdFrCqaS94fXX9LuKh \
  --cluster Mainnet \
  --concurrency 50 \
  --block-retry-delay <BLOCK_RETRY_DELAY>
```

**For monitoring and paying all pye-accounts for given validator and specific issuers**
```sh
./target/release/pye-cli validator-pye-account-manager \
  --rpc https://api.mainnet-beta.solana.com \
  --payer ~/.config/solana/id.json \
  --vote-pubkey <VALIDATOR_VOTE_PUBKEY> \
  --issuers <YOUR_ISSUER_PUBKEY> \
  --issuers <ANOTHER_ISSUER_PUBKEY> \
  --concurrency 10 \
  --block-retry-delay <BLOCK_RETRY_DELAY>
```

## Monitoring

1. (For local monitoring) Setup an instance of InfluxDB and Grafana with `docker-compose up -d` (Pre-requisite: Docker installation).
2. Set env with `export SOLANA_METRICS_CONFIG="host=http://localhost:8086,db=metrics,u=admin,p=admin"` and `export RUST_LOG=info,solana_metrics=warn`. Replace host with endpoint of remote InfluxDB if using.
3. Run the CLI normally — metrics will be automatically logged to InfluxDB.

### Reading Metrics

There are several ways to read the metrics logged. For instance, we can use the InfluxDB CLI:

1. Get container id using `docker ps`, e.g. `pye-program-library-influxdb-1`
2. Connect to InfluxDB container with `docker exec -it pye-cli-influxdb-1 influx`.
3. Run the following

```
$ USE metrics
$ SELECT * FROM excess_reward ORDER BY time DESC LIMIT 50;
$ SELECT * FROM validator_mev_data ORDER BY time DESC LIMIT 50;
$ SELECT * FROM reward_commissions ORDER BY time DESC LIMIT 50;
```
