use bdk_electrum::{BdkElectrumClient, electrum_client, electrum_client::Client};
use bdk_wallet::{
    AddressInfo, KeychainKind, Wallet,
    bitcoin::{Network, NetworkKind, bip32::Xpriv, key::rand::{self, RngCore}},
    chain::spk_client::{SyncRequest, SyncRequestBuilder}, template::{Bip86, DescriptorTemplate},
};

const ELECTRUM_SERVER: &str = "tcp://localhost:50001";
const STOP_GAP: usize = 50;
const BATCH_SIZE: usize = 5;

fn main() {
    use std::io::{self, Write};

    loop {
        println!("\n====== メニュー ======");
        println!("1. create_seed()を実行");
        println!("2. seedからcreate_wallet()を実行");
        println!("3. 終了");
        println!("====================");
        print!("選択を入力してください (1-3): ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).expect("入力の読み込みに失敗しました");
        let choice = input.trim();

        match choice {
            "1" => create_seed(),
            "2" => {
                print!("seedを入力してください: ");
                io::stdout().flush().unwrap();

                let mut seed = String::new();
                io::stdin()
                    .read_line(&mut seed)
                    .expect("seedの読み込みに失敗しました");
                let seed = seed.trim();

                create_wallet(seed);
            }
            "3" => {
                println!("終了します。");
                break;
            }
            _ => println!("無効な入力です。1、2、または3を入力してください。"),
        }
    }
}

fn create_seed() {
    let mut seed: [u8; 32] = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let seed_hex = hex::encode(seed);
    println!("seed: {}", seed_hex);
}

// https://bitcoindevkit.github.io/book-of-bdk/cookbook/keys-descriptors/descriptors/#using-descriptor-templates
fn create_priv(seed_hex: &str) -> (String, String) {
    let mut seed: [u8; 32] = [0u8; 32];
    hex::decode_to_slice(seed_hex, &mut seed).expect("Invalid seed hex");

    let network: Network = Network::Signet;
    let kind = if network == Network::Bitcoin {
        NetworkKind::Main
    } else {
        NetworkKind::Test
    };
    let xprv: Xpriv = Xpriv::new_master(network, &seed).unwrap();
    let (descriptor, key_map, _) = Bip86(xprv, KeychainKind::External)
        .build(kind)
        .expect("Failed to build external descriptor");

    let (change_descriptor, change_key_map, _) = Bip86(xprv, KeychainKind::Internal)
        .build(kind)
        .expect("Failed to build internal descriptor");

    let descriptor_string_priv = descriptor.to_string_with_secret(&key_map);
    let change_descriptor_string_priv = change_descriptor.to_string_with_secret(&change_key_map);
    (descriptor_string_priv, change_descriptor_string_priv)
}

fn create_wallet(seed_hex: &str) {
    let (external_descriptor, internal_descriptor) = create_priv(seed_hex);
    let mut wallet: Wallet = Wallet::create(external_descriptor, internal_descriptor)
        .network(Network::Regtest)
        .create_wallet_no_persist()
        .expect("Failed to create wallet");

    let address: AddressInfo = wallet.reveal_next_address(KeychainKind::External);
    println!(
        "Generated address {} at index {}",
        address.address, address.index
    );

    // Create the Electrum client
    let client: BdkElectrumClient<Client> = BdkElectrumClient::new(
        electrum_client::Client::new(ELECTRUM_SERVER).expect("Failed to create Electrum client"),
    );

    // Perform the initial full scan on the wallet
    println!("full_scanning...");
    let start = std::time::Instant::now();
    let full_scan_request = wallet.start_full_scan();
    let update = client
        .full_scan(full_scan_request, STOP_GAP, BATCH_SIZE, true)
        .expect("Failed to perform full scan");

    wallet.apply_update(update).expect("Failed to apply update");
    let duration = start.elapsed();
    println!("full_scan elapsed: {:?}", duration);

    loop {
        std::thread::sleep(std::time::Duration::from_secs(10));

        let start = std::time::Instant::now();
        let sync_request = sync_request(&wallet);
        let sync_response = client
            .sync(sync_request, BATCH_SIZE, false)
            .expect("Failed to sync");
        wallet
            .apply_update(sync_response)
            .expect("Failed to apply update");
        let duration = start.elapsed();
        println!("sync elapsed: {:?}", duration);

        let balance = wallet.balance();
        println!("Wallet balance: {} sat\n", balance.total().to_sat());
    }
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
