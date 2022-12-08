use tower::{Service, ServiceExt};
use tower::limit::ConcurrencyLimit;
use tracing::info;
use tonlibjson_client::config::AppConfig;
use tonlibjson_client::cursor_client::CursorClient;
use tonlibjson_client::make::ClientFactory;
use tonlibjson_client::session::SessionClient;
use tonlibjson_client::ton_config::load_ton_config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let app_config = AppConfig::from_env()?;
    let ton_config = load_ton_config(app_config.config_url).await?;

    let mut client = ClientFactory::default()
        .ready()
        .await?
        .call(ton_config)
        .await?;

    let p = client.ready().await?;

    info!("start seqno: {:?}, end seqno: {:?}",
        p.first_block().expect("must be synced").id.seqno,
        p.last_block().expect("must be synced").id.seqno
    );

    info!("header: {:?}", p.first_block().unwrap());

    Ok(())
}
