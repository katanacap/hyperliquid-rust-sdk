use hyperliquid_rust_sdk::{BaseUrl, InfoClient};
use log::info;

#[tokio::main]
async fn main() {
    env_logger::init();
    let info_client = InfoClient::new(None, Some(BaseUrl::Mainnet)).await.unwrap();

    perp_meta_example(&info_client).await;
}

async fn perp_meta_example(info_client: &InfoClient) {
    // println!("Perp metadata: {:?}", info_client.meta_and_asset_contexts().await.unwrap());
    info!(
        "Perp metadata: {:?}",
        info_client.meta_and_asset_contexts().await.unwrap()
    );
}
