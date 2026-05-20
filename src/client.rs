// SPDX-License-Identifier: Apache-2.0

mod live;
mod plot;

use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use clap::{Parser, Subcommand};
use efbench::proto::{Ping, benchmark_client::BenchmarkClient};
use tokio::sync::mpsc;

struct BenchState {
    total_latency_us: u128,
    req_count: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct Metrics {
    pub(crate) avg_latency_us: f64,
    pub(crate) req_count: u64,
    pub(crate) rx_bytes: u64,
    pub(crate) tx_bytes: u64,
}

#[derive(Parser)]
#[command(name = "efbench-client", about = "Benchmark client for efense")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Live TUI benchmark
    Live {
        #[arg(long)]
        iface: String,
        #[arg(long)]
        ip: String,
        #[arg(long)]
        port: u16,
    },
    /// Run benchmark and save results
    Plot {
        #[arg(long)]
        iface: String,
        #[arg(long)]
        ip: String,
        #[arg(long)]
        port: u16,
        #[arg(long, default_value = "benchmark_output")]
        output: String,
    },
}

fn check_interface(iface: &str) -> anyhow::Result<()> {
    let path = format!("/sys/class/net/{iface}");
    if !std::path::Path::new(&path).is_dir() {
        anyhow::bail!("network interface '{iface}' not found at {path}");
    }
    Ok(())
}

async fn check_connection(server_addr: &str) -> anyhow::Result<()> {
    BenchmarkClient::connect(format!("http://{server_addr}"))
        .await
        .map_err(|e| {
            anyhow::anyhow!("Failed to connect to server at {server_addr}: {e}")
        })?;
    Ok(())
}

fn read_net_stat(iface: &str, stat: &str) -> u64 {
    let path = format!("/sys/class/net/{iface}/statistics/{stat}");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

async fn benchmark_loop(
    server_addr: String,
    _iface: String,
    state: Arc<Mutex<BenchState>>,
    stop: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client =
        BenchmarkClient::connect(format!("http://{server_addr}")).await?;

    let (req_tx, req_rx) = mpsc::channel::<Ping>(1000);
    let outbound = tokio_stream::wrappers::ReceiverStream::new(req_rx);

    let response = client.ping_pong(tonic::Request::new(outbound)).await?;
    let mut inbound = response.into_inner();

    let mut seq = 0u64;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let sent = Instant::now();
        req_tx.send(Ping { seq, sent_ns: 0 }).await?;

        match inbound.message().await {
            Ok(Some(_)) => {
                let latency = sent.elapsed();
                if let Ok(mut s) = state.lock() {
                    s.total_latency_us += latency.as_micros();
                    s.req_count += 1;
                }
                seq += 1;
            }
            Ok(None) => break,
            Err(e) => {
                eprintln!("gRPC error: {e}");
                break;
            }
        }
    }

    Ok(())
}

async fn reporter_task(
    iface: String,
    state: Arc<Mutex<BenchState>>,
    metrics_tx: mpsc::Sender<Metrics>,
    stop: Arc<AtomicBool>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut last_rx = read_net_stat(&iface, "rx_bytes");
    let mut last_tx = read_net_stat(&iface, "tx_bytes");

    loop {
        interval.tick().await;

        if stop.load(Ordering::Relaxed) {
            break;
        }

        let now_rx = read_net_stat(&iface, "rx_bytes");
        let now_tx = read_net_stat(&iface, "tx_bytes");

        let (avg_latency_us, req_count) = {
            if let Ok(mut s) = state.lock() {
                let avg = if s.req_count > 0 {
                    s.total_latency_us as f64 / s.req_count as f64
                } else {
                    0.0
                };
                let cnt = s.req_count;
                s.total_latency_us = 0;
                s.req_count = 0;
                (avg, cnt)
            } else {
                (0.0, 0)
            }
        };

        let _ = metrics_tx
            .send(Metrics {
                avg_latency_us,
                req_count,
                rx_bytes: now_rx.saturating_sub(last_rx),
                tx_bytes: now_tx.saturating_sub(last_tx),
            })
            .await;

        last_rx = now_rx;
        last_tx = now_tx;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Live { iface, ip, port } => {
            check_interface(&iface)?;
            let server_addr = format!("{ip}:{port}");
            check_connection(&server_addr).await?;
            let stop = Arc::new(AtomicBool::new(false));
            let state = Arc::new(Mutex::new(BenchState {
                total_latency_us: 0,
                req_count: 0,
            }));
            let app_data =
                Arc::new(Mutex::new(live::AppData::new(ip.clone(), port)));

            let (metrics_tx, metrics_rx) = mpsc::channel::<Metrics>(16);

            // Spawn benchmark task
            let b_state = state.clone();
            let b_stop = stop.clone();
            let b_addr = server_addr.clone();
            let b_iface = iface.clone();
            let bench_handle = tokio::spawn(async move {
                if let Err(e) =
                    benchmark_loop(b_addr, b_iface, b_state, b_stop).await
                {
                    eprintln!("Benchmark error: {e}");
                }
            });

            // Spawn reporter task
            let r_state = state.clone();
            let r_stop = stop.clone();
            let r_iface = iface.clone();
            let r_tx = metrics_tx.clone();
            let report_handle = tokio::spawn(async move {
                reporter_task(r_iface, r_state, r_tx, r_stop).await;
            });

            // Run TUI on main thread (actually this is inside tokio::main, but
            // we use spawn_blocking for the synchronous TUI)
            let t_app_data = app_data.clone();
            let t_stop = stop.clone();
            let t_handle = tokio::task::spawn_blocking(move || {
                live::run_tui(metrics_rx, t_app_data, t_stop)
            });

            t_handle.await??;

            stop.store(true, Ordering::Relaxed);
            let _ = bench_handle.await;
            let _ = report_handle.await;
        }
        Command::Plot {
            iface,
            ip,
            port,
            output,
        } => {
            check_interface(&iface)?;
            let server_addr = format!("{ip}:{port}");
            check_connection(&server_addr).await?;
            let stop = Arc::new(AtomicBool::new(false));
            let state = Arc::new(Mutex::new(BenchState {
                total_latency_us: 0,
                req_count: 0,
            }));

            let (metrics_tx, mut metrics_rx) = mpsc::channel::<Metrics>(4096);

            let b_state = state.clone();
            let b_stop = stop.clone();
            let b_addr = server_addr.clone();
            let b_iface = iface.clone();
            let bench_handle = tokio::spawn(async move {
                if let Err(e) =
                    benchmark_loop(b_addr, b_iface, b_state, b_stop).await
                {
                    eprintln!("Benchmark error: {e}");
                }
            });

            let r_state = state.clone();
            let r_stop = stop.clone();
            let r_iface = iface.clone();
            let r_tx = metrics_tx.clone();
            let report_handle = tokio::spawn(async move {
                reporter_task(r_iface, r_state, r_tx, r_stop).await;
            });

            let mut all_metrics: Vec<Metrics> = Vec::new();
            let ctrlc_stop = stop.clone();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                ctrlc_stop.store(true, Ordering::Relaxed);
            });

            println!("Benchmark running... Press Ctrl-C to stop.");

            // Collect metrics until stop
            loop {
                tokio::select! {
                    Some(m) = metrics_rx.recv() => {
                        all_metrics.push(m);
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if stop.load(Ordering::Relaxed) {
                            // Drain remaining
                            while let Ok(m) = metrics_rx.try_recv() {
                                all_metrics.push(m);
                            }
                            break;
                        }
                    }
                }
            }

            stop.store(true, Ordering::Relaxed);
            let _ = bench_handle.await;
            let _ = report_handle.await;

            plot::save_plot(&all_metrics, &output)?;
        }
    }

    Ok(())
}
