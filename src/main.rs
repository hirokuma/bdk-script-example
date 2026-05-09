use std::{collections::BTreeMap, str::FromStr};

use bdk_electrum::{BdkElectrumClient, electrum_client, electrum_client::Client};
use bdk_wallet::{
    AddressInfo, Balance, KeychainKind, Wallet,
    bitcoin::{
        Address, Amount, Network, NetworkKind, Psbt,
        bip32::Xpriv,
        consensus::encode,
        key::rand::{self, RngCore},
    },
    chain::spk_client::{SyncRequest, SyncRequestBuilder},
    descriptor,
    miniscript::{DescriptorPublicKey, ForEachKey},
    template::{Bip86, DescriptorTemplate},
};

const ELECTRUM_SERVER: &str = "tcp://localhost:50001";
const STOP_GAP: usize = 50;
const BATCH_SIZE: usize = 5;
const NETWORK: Network = Network::Regtest;
const WATCH_DURATION_SECS: u64 = 1;

// pick as internal key a "Nothing Up My Sleeve" (NUMS) point
// TODO H + rG
// https://github.com/bitcoin/bips/blob/master/bip-0341.mediawiki#constructing-and-spending-taproot-outputs
const NUMS_XPUBKEY: &str = "50929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac0";

fn main() -> anyhow::Result<()> {
    let mut wallet1 = create_wallet(create_seed().as_str());
    let xpub1 = wallet1.public_descriptor(KeychainKind::External);
    let xpub1i = wallet1.public_descriptor(KeychainKind::Internal);
    let mut wallet2 = create_wallet(create_seed().as_str());
    let xpub2 = wallet2.public_descriptor(KeychainKind::External);
    let xpub2i = wallet2.public_descriptor(KeychainKind::Internal);

    let descriptor = create_multisig_descriptor(&xpub1, &xpub2);
    let descriptor_i = create_multisig_descriptor(&xpub1i, &xpub2i);

    // create watch only wallet with the multisig descriptor
    let mut multi_wallet = Wallet::create(descriptor, descriptor_i)
        .network(NETWORK)
        .create_wallet_no_persist()
        .expect("Failed to create wallet");

    full_scan(&mut multi_wallet);

    loop {
        let address = get_new_address(&mut multi_wallet);
        println!("Generated address: {}", address);
        let balance = watch_wallet(&mut multi_wallet);

        let address = get_new_address(&mut multi_wallet);
        let send_sat = balance.total().to_sat() / 2;
        println!("Send to address(amount={} sat): {}", send_sat, address);
        send_to_address(
            &mut multi_wallet,
            &mut wallet1,
            &mut wallet2,
            &address,
            send_sat,
        );
        let _ = watch_wallet(&mut multi_wallet);

        println!("------------------------------");
    }
}

fn create_seed() -> String {
    let mut seed: [u8; 32] = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    hex::encode(seed)
}

// https://bitcoindevkit.github.io/book-of-bdk/cookbook/keys-descriptors/descriptors/#using-descriptor-templates
fn create_wallet(seed_hex: &str) -> Wallet {
    let mut seed: [u8; 32] = [0u8; 32];
    hex::decode_to_slice(seed_hex, &mut seed).expect("Invalid seed hex");

    let network: Network = Network::Signet;
    let kind = NetworkKind::from(network);
    let xprv: Xpriv = Xpriv::new_master(network, &seed).unwrap();
    let (descriptor, key_map, _) = Bip86(xprv, KeychainKind::External)
        .build(kind)
        .expect("Failed to build external descriptor");

    let (change_descriptor, change_key_map, _) = Bip86(xprv, KeychainKind::Internal)
        .build(kind)
        .expect("Failed to build internal descriptor");

    let external_descriptor_priv = descriptor.to_string_with_secret(&key_map);
    let internal_descriptor_priv = change_descriptor.to_string_with_secret(&change_key_map);
    Wallet::create(external_descriptor_priv, internal_descriptor_priv)
        .network(NETWORK)
        .create_wallet_no_persist()
        .expect("Failed to create wallet")
}

fn create_multisig_descriptor(
    xpub1: &descriptor::Descriptor<DescriptorPublicKey>,
    xpub2: &descriptor::Descriptor<DescriptorPublicKey>,
) -> descriptor::Descriptor<DescriptorPublicKey> {
    let key1_ext = convert_descriptor(xpub1).expect("Failed to extract xpub from wallet 1");
    let key2_ext = convert_descriptor(xpub2).expect("Failed to extract xpub from wallet 2");
    let intr_ext = DescriptorPublicKey::from_str(NUMS_XPUBKEY)
        .inspect_err(|e| eprintln!("Internal key error: {}", e))
        .unwrap();
    let (descriptor, _, _) = descriptor! {
        tr(intr_ext, multi_a(2, key1_ext, key2_ext))
    }
    .inspect_err(|e| eprintln!("Descriptor error: {}", e))
    .unwrap();
    descriptor
}

fn convert_descriptor(
    xpub: &descriptor::Descriptor<DescriptorPublicKey>,
) -> Option<DescriptorPublicKey> {
    let mut key_ext = None;
    xpub.for_each_key(|key| {
        key_ext = Some(key.clone());
        false
    });
    key_ext
}

fn get_new_address(wallet: &mut Wallet) -> String {
    let address: AddressInfo = wallet.reveal_next_address(KeychainKind::External);
    address.address.to_string()
}

fn full_scan(wallet: &mut Wallet) {
    let client: BdkElectrumClient<Client> = BdkElectrumClient::new(
        electrum_client::Client::new(ELECTRUM_SERVER).expect("Failed to create Electrum client"),
    );

    let full_scan_request = wallet.start_full_scan();
    let update = client
        .full_scan(full_scan_request, STOP_GAP, BATCH_SIZE, true)
        .expect("Failed to perform full scan");
    wallet.apply_update(update).expect("Failed to apply update");
}

fn watch_wallet(wallet: &mut Wallet) -> Balance {
    let client: BdkElectrumClient<Client> = BdkElectrumClient::new(
        electrum_client::Client::new(ELECTRUM_SERVER).expect("Failed to create Electrum client"),
    );

    let mut balance = wallet.balance();
    println!("Initial wallet balance: {} sat", balance.total().to_sat());
    loop {
        std::thread::sleep(std::time::Duration::from_secs(WATCH_DURATION_SECS));

        let sync_request = sync_request(&wallet);
        let sync_response = client
            .sync(sync_request, BATCH_SIZE, false)
            .expect("Failed to sync");
        wallet
            .apply_update(sync_response)
            .expect("Failed to apply update");

        let new_balance = wallet.balance();
        if new_balance.total() != balance.total() {
            println!("Wallet balance: {} sat", new_balance.total().to_sat());
            balance = new_balance;
            break;
        }
    }
    balance
}

fn sync_request(wallet: &Wallet) -> SyncRequestBuilder<(bdk_wallet::KeychainKind, u32)> {
    let mut spks_to_sync = std::collections::BTreeSet::new();

    // Externalアドレスのみチェックする
    if let Some(derived_index) = wallet.derivation_index(KeychainKind::External) {
        for index in 0..=derived_index {
            let address_info = wallet.peek_address(KeychainKind::External, index);
            spks_to_sync.insert((
                (KeychainKind::External, index),
                address_info.address.script_pubkey(),
            ));
        }
    }
    for tx in wallet.transactions() {
        if tx.chain_position.is_unconfirmed() {
            for out in &tx.tx_node.tx.output {
                if let Some(index) = wallet.spk_index().index_of_spk(out.script_pubkey.clone()) {
                    spks_to_sync.insert((*index, out.script_pubkey.clone()));
                }
            }
        }
    }
    let chain_tip = wallet.local_chain().tip();
    SyncRequest::builder()
        .chain_tip(chain_tip)
        .spks_with_indexes(spks_to_sync)
}

fn send_to_address(
    multi_wallet: &mut Wallet,
    wallet1: &mut Wallet,
    wallet2: &mut Wallet,
    address: &str,
    amount_sat: u64,
) {
    let mut psbt = create_psbt(multi_wallet, address, amount_sat).expect("Failed to create PSBT");

    let _ = wallet1
        .sign(
            &mut psbt,
            bdk_wallet::SignOptions {
                trust_witness_utxo: true,
                try_finalize: false,
                ..Default::default()
            },
        )
        .expect("Failed to sign PSBT with wallet 1");
    let _ = wallet2
        .sign(
            &mut psbt,
            bdk_wallet::SignOptions {
                trust_witness_utxo: true,
                try_finalize: false,
                ..Default::default()
            },
        )
        .expect("Failed to sign PSBT with wallet 2");
    let finalized = multi_wallet
        .finalize_psbt(
            &mut psbt,
            bdk_wallet::SignOptions {
                trust_witness_utxo: true,
                try_finalize: true,
                ..Default::default()
            },
        )
        .expect("Failed to finalize PSBT");
    if !finalized {
        eprintln!("Failed to finalize PSBT");
        return;
    }
    let tx = psbt
        .extract_tx()
        .expect("Failed to extract transaction from PSBT");
    println!("Final transaction: {}", encode::serialize_hex(&tx));
}

fn create_psbt(wallet: &mut Wallet, address: &str, amount_sat: u64) -> anyhow::Result<Psbt> {
    let wallet_policy = wallet.policies(KeychainKind::External)?;
    let wallet_policy = match wallet_policy {
        Some(v) => v,
        None => {
            let msg = "no wallet policy";
            eprintln!("{msg}");
            anyhow::bail!(msg)
        }
    };

    let mut builder = wallet.build_tx();
    builder.only_witness_utxo();

    let address = Address::from_str(address)?.require_network(NETWORK)?;
    builder.add_recipient(address.script_pubkey(), Amount::from_sat(amount_sat));

    let mut path = BTreeMap::new();
    path.insert(wallet_policy.id, vec![1]);
    builder.policy_path(path.clone(), KeychainKind::External);
    builder.policy_path(path, KeychainKind::Internal);

    Ok(builder.finish()?)
}
