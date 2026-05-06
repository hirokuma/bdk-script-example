use bdk_electrum::{BdkElectrumClient, electrum_client, electrum_client::Client};
use bdk_wallet::{
    AddressInfo, KeychainKind, Wallet,
    bitcoin::Network,
    chain::spk_client::{SyncRequest, SyncRequestBuilder},
};

const ELECTRUM_SERVER: &str = "tcp://localhost:50001";
const STOP_GAP: usize = 50;
const BATCH_SIZE: usize = 5;
const EXTERNAL_DESCRIPTOR: &str = "tr(tprv8ZgxMBicQKsPdrjwWCyXqqJ4YqcyG4DmKtjjsRt29v1PtD3r3PuFJAjWytzcvSTKnZAGAkPSmnrdnuHWxCAwy3i1iPhrtKAfXRH7dVCNGp6/86'/1'/0'/0/*)#g9xn7wf9";
const INTERNAL_DESCRIPTOR: &str = "tr(tprv8ZgxMBicQKsPdrjwWCyXqqJ4YqcyG4DmKtjjsRt29v1PtD3r3PuFJAjWytzcvSTKnZAGAkPSmnrdnuHWxCAwy3i1iPhrtKAfXRH7dVCNGp6/86'/1'/0'/1/*)#e3rjrmea";

fn main() {
    let mut wallet: Wallet = Wallet::create(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
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
