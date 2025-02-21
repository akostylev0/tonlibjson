mod account;
mod block;
mod helpers;
mod message;
#[allow(clippy::enum_variant_names)]
mod ton;

use crate::account::AccountService;
use crate::block::BlockService;
use crate::message::MessageService;
use crate::ton::account_service_server::AccountServiceServer;
use crate::ton::block_service_server::BlockServiceServer;
use crate::ton::message_service_server::MessageServiceServer;
use clap::Parser;
use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;
use std::time::Duration;
use tonic::codec::CompressionEncoding::Gzip;
use tonic::transport::Server;
use tonlibjson_client::ton::TonClientBuilder;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::EnvFilter;
use url::Url;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[clap(long, default_value = "0.0.0.0:50052")]
    listen: SocketAddr,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "30s")]
    timeout: Duration,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "300s")]
    tcp_keepalive: Duration,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "120s")]
    http2_keepalive_interval: Duration,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "20s")]
    http2_keepalive_timeout: Duration,
    #[clap(long, default_value = "65535")]
    initial_connection_window_size: u32,
    #[clap(long, default_value = "65535")]
    initial_stream_window_size: u32,

    #[clap(long)]
    enable_metrics: bool,
    #[clap(long, default_value = "0.0.0.0:9000")]
    metrics_listen: SocketAddr,

    #[clap(long, value_parser = Url::parse, default_value_t = tonlibjson_client::ton::default_ton_config_url())]
    ton_config_url: Url,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "10s")]
    ton_timeout: Duration,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "10s")]
    retry_budget_ttl: Duration,
    #[clap(long, default_value_t = 1)]
    retry_min_rps: u32,
    #[clap(long, default_value_t = 0.1)]
    retry_withdraw_percent: f32,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "128ms")]
    retry_first_delay: Duration,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "4096ms")]
    retry_max_delay: Duration,

    #[clap(long, value_parser = humantime::parse_duration, default_value = "70ms")]
    ewma_default_rtt: Duration,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "1ms")]
    ewma_decay: Duration,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_span_events(FmtSpan::CLOSE)
        .init();

    if args.enable_metrics {
        PrometheusBuilder::new()
            .with_http_listener(args.metrics_listen)
            .install()
            .expect("failed to install Prometheus recorder");

        tracing::info!("Listening metrics on {:?}", &args.metrics_listen);
    }

    tracing::info!("TON Config URL: {}", &args.ton_config_url);

    let mut client =
        TonClientBuilder::from_config_url(args.ton_config_url, Duration::from_secs(60))
            .set_timeout(args.ton_timeout)
            .set_retry_budget_ttl(args.retry_budget_ttl)
            .set_retry_min_per_sec(args.retry_min_rps)
            .set_retry_percent(args.retry_withdraw_percent)
            .set_retry_first_delay(args.retry_first_delay)
            .set_retry_max_delay(args.retry_max_delay)
            .set_ewma_default_rtt(args.ewma_default_rtt)
            .set_ewma_decay(args.ewma_decay)
            .build()?;

    client.ready().await?;
    tracing::info!("Ton Client is ready");

    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .register_encoded_file_descriptor_set(ton::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    let account_service = AccountServiceServer::new(AccountService::new(client.clone()))
        .accept_compressed(Gzip)
        .send_compressed(Gzip);
    let block_service = BlockServiceServer::new(BlockService::new(client.clone()))
        .accept_compressed(Gzip)
        .send_compressed(Gzip);
    let message_service = MessageServiceServer::new(MessageService::new(client))
        .accept_compressed(Gzip)
        .send_compressed(Gzip);

    let (mut health_reporter, health_server) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<AccountServiceServer<AccountService>>()
        .await;
    health_reporter
        .set_serving::<BlockServiceServer<BlockService>>()
        .await;
    health_reporter
        .set_serving::<MessageServiceServer<MessageService>>()
        .await;

    tracing::info!("Listening on {:?}", &args.listen);

    Server::builder()
        .timeout(args.timeout)
        .tcp_keepalive(args.tcp_keepalive.into())
        .http2_keepalive_interval(args.http2_keepalive_interval.into())
        .http2_keepalive_timeout(args.http2_keepalive_timeout.into())
        .initial_connection_window_size(args.initial_connection_window_size)
        .initial_stream_window_size(args.initial_stream_window_size)
        .add_service(reflection)
        .add_service(health_server)
        .add_service(account_service)
        .add_service(block_service)
        .add_service(message_service)
        .serve_with_shutdown(args.listen, async move {
            tokio::signal::ctrl_c().await.unwrap();
        })
        .await?;

    Ok(())
}
