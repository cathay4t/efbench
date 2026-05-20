// SPDX-License-Identifier: Apache-2.0

use std::{net::SocketAddr, pin::Pin};

use clap::Parser;
use efbench::proto::{
    Ping, Pong,
    benchmark_server::{Benchmark, BenchmarkServer},
};
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status, transport::Server};

#[derive(Debug, Default)]
struct BenchmarkService;

#[tonic::async_trait]
impl Benchmark for BenchmarkService {
    type PingPongStream =
        Pin<Box<dyn Stream<Item = Result<Pong, Status>> + Send + 'static>>;

    async fn ping_pong(
        &self,
        request: Request<tonic::Streaming<Ping>>,
    ) -> Result<Response<Self::PingPongStream>, Status> {
        let mut stream = request.into_inner();

        let output = async_stream::try_stream! {
            let data = vec![0u8; 16 * 1024];
            while let Some(ping) = stream.next().await {
                ping?;
                yield Pong { data: data.clone() };
            }
        };

        Ok(Response::new(Box::pin(output) as Self::PingPongStream))
    }
}

#[derive(Parser)]
struct Args {
    ip: String,
    port: u16,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let addr: SocketAddr = format!("{}:{}", args.ip, args.port).parse()?;

    println!("efbench-server listening on {addr}");

    Server::builder()
        .add_service(BenchmarkServer::new(BenchmarkService::default()))
        .serve(addr)
        .await?;

    Ok(())
}
