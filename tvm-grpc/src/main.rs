mod threaded;
mod transaction_emulator;
#[allow(clippy::enum_variant_names)]
mod tvm;
mod tvm_emulator;

use crate::transaction_emulator::TransactionEmulatorService;
use crate::tvm::transaction_emulator_service_server::TransactionEmulatorServiceServer;
use crate::tvm::tvm_emulator_service_server::TvmEmulatorServiceServer;
use crate::tvm_emulator::TvmEmulatorService;
use clap::Parser;
use std::net::SocketAddr;
use std::time::Duration;
use tonic::codec::CompressionEncoding::Gzip;
use tonic::transport::Server;
use tonlibjson_sys::TvmEmulator;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::EnvFilter;

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    TvmEmulator::set_verbosity_level(0);

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_span_events(FmtSpan::CLOSE)
        .init();

    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .register_encoded_file_descriptor_set(tvm::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    let (mut health_reporter, health_server) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<TvmEmulatorServiceServer<TvmEmulatorService>>()
        .await;
    health_reporter
        .set_serving::<TransactionEmulatorServiceServer<TransactionEmulatorService>>()
        .await;

    let tvm_emulator_service = TvmEmulatorServiceServer::new(TvmEmulatorService::default())
        .accept_compressed(Gzip)
        .send_compressed(Gzip);
    let transaction_emulator_service =
        TransactionEmulatorServiceServer::new(TransactionEmulatorService::default())
            .accept_compressed(Gzip)
            .send_compressed(Gzip);

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
        .add_service(tvm_emulator_service)
        .add_service(transaction_emulator_service)
        .serve_with_shutdown(args.listen, async move {
            tokio::signal::ctrl_c().await.unwrap();
        })
        .await?;

    Ok(())
}
