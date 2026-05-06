// original: https://github.com/bitcoindevkit/book-of-bdk/blob/474aa61ecd444833354e4e1177000d90362f1471/examples/rust/syncing/electrum/src/main.rs
use bdk_electrum::electrum_client::Client;
use bdk_electrum::{electrum_client, BdkElectrumClient};
use bdk_wallet::bitcoin::Network;
use bdk_wallet::chain::spk_client::{SyncRequest, SyncRequestBuilder};
use bdk_wallet::AddressInfo;
use bdk_wallet::KeychainKind;
use bdk_wallet::Wallet;

const STOP_GAP: usize = 50;
const BATCH_SIZE: usize = 5;
const EXTERNAL_DESCRIPTOR: &str = "tr(tprv8ZgxMBicQKsPdrjwWCyXqqJ4YqcyG4DmKtjjsRt29v1PtD3r3PuFJAjWytzcvSTKnZAGAkPSmnrdnuHWxCAwy3i1iPhrtKAfXRH7dVCNGp6/86'/1'/0'/0/*)#g9xn7wf9";
const INTERNAL_DESCRIPTOR: &str = "tr(tprv8ZgxMBicQKsPdrjwWCyXqqJ4YqcyG4DmKtjjsRt29v1PtD3r3PuFJAjWytzcvSTKnZAGAkPSmnrdnuHWxCAwy3i1iPhrtKAfXRH7dVCNGp6/86'/1'/0'/1/*)#e3rjrmea";

fn main() {
    let mut wallet: Wallet = Wallet::create(EXTERNAL_DESCRIPTOR, INTERNAL_DESCRIPTOR)
        .network(Network::Signet)
        .create_wallet_no_persist()
        .unwrap();

    let address: AddressInfo = wallet.reveal_next_address(KeychainKind::External);
    println!(
        "Generated address {} at index {}",
        address.address, address.index
    );

    // Create the Electrum client
    let client: BdkElectrumClient<Client> =
        BdkElectrumClient::new(electrum_client::Client::new("ssl://mempool.space:60602").unwrap());

    // Perform the initial full scan on the wallet
    println!("full_scanning...");
    let start = std::time::Instant::now();
    let full_scan_request = wallet.start_full_scan();
    let update = client
        .full_scan(full_scan_request, STOP_GAP, BATCH_SIZE, true)
        .unwrap();

    wallet.apply_update(update).unwrap();
    let duration = start.elapsed();
    println!("full_scan elapsed: {:?}", duration);

    let balance = wallet.balance();
    println!("Wallet balance: {} sat", balance.total().to_sat());

    loop {
        std::thread::sleep(std::time::Duration::from_secs(10));

        let start = std::time::Instant::now();
        let sync_request = sync_request(&wallet);
        let sync_response = client.sync(sync_request, BATCH_SIZE, false).unwrap();
        wallet.apply_update(sync_response).unwrap();
        let duration = start.elapsed();
        println!("sync elapsed: {:?}", duration);
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
                if let Some(index) = wallet
                    .spk_index()
                    .index_of_spk(out.script_pubkey.clone())
                {
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
