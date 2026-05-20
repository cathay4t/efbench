// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style, Stylize},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph},
};
use tokio::sync::mpsc;

use crate::Metrics;

const MAX_POINTS: usize = 120;

#[derive(Clone)]
pub struct AppData {
    pub ip: String,
    pub port: u16,
    latencies: VecDeque<f64>,
    throughputs: VecDeque<f64>,
    rx_rates: VecDeque<f64>,
    tx_rates: VecDeque<f64>,
    pub current_latency: f64,
    pub current_throughput: u64,
    pub current_rx: u64,
    pub current_tx: u64,
    pub total_reqs: u64,
    pub elapsed: u64,
    pub connected: bool,
}

impl AppData {
    pub fn new(ip: String, port: u16) -> Self {
        Self {
            ip,
            port,
            latencies: VecDeque::with_capacity(MAX_POINTS),
            throughputs: VecDeque::with_capacity(MAX_POINTS),
            rx_rates: VecDeque::with_capacity(MAX_POINTS),
            tx_rates: VecDeque::with_capacity(MAX_POINTS),
            current_latency: 0.0,
            current_throughput: 0,
            current_rx: 0,
            current_tx: 0,
            total_reqs: 0,
            elapsed: 0,
            connected: false,
        }
    }

    pub fn push_metrics(&mut self, m: Metrics) {
        self.current_latency = m.avg_latency_us;
        self.current_throughput = m.req_count;
        self.current_rx = m.rx_bytes;
        self.current_tx = m.tx_bytes;

        if self.latencies.len() >= MAX_POINTS {
            self.latencies.pop_front();
        }
        self.latencies.push_back(self.current_latency);

        if self.throughputs.len() >= MAX_POINTS {
            self.throughputs.pop_front();
        }
        self.throughputs.push_back(self.current_throughput as f64);

        if self.rx_rates.len() >= MAX_POINTS {
            self.rx_rates.pop_front();
        }
        self.rx_rates.push_back(m.rx_bytes as f64);

        if self.tx_rates.len() >= MAX_POINTS {
            self.tx_rates.pop_front();
        }
        self.tx_rates.push_back(m.tx_bytes as f64);

        self.total_reqs += m.req_count;
        self.elapsed += 1;
    }
}

fn make_chart<'a>(
    title: &'a str,
    datasets: Vec<Dataset<'a>>,
    _y_label: &'a str,
    y_max: f64,
) -> Chart<'a> {
    let y_max = if y_max <= 0.0 { 1.0 } else { y_max * 1.1 };

    Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .title_alignment(Alignment::Center),
        )
        .x_axis(
            Axis::default()
                .bounds([0.0, MAX_POINTS as f64])
                .labels(vec![
                    Span::raw("0"),
                    Span::raw(format!("{}s", MAX_POINTS / 2)),
                    Span::raw(format!("{}s", MAX_POINTS)),
                ])
                .style(Style::default().gray()),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, y_max])
                .style(Style::default().gray()),
        )
}

fn draw_ui(frame: &mut ratatui::Frame, app: &AppData) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let header = format!(
        " {}:{}  |  Latency: {:.0}μs  |  Throughput: {} req/s  |  RX: {:.1} \
         MiB/s  TX: {:.1} MiB/s",
        app.ip,
        app.port,
        app.current_latency,
        app.current_throughput,
        app.current_rx as f64 / (1024.0 * 1024.0),
        app.current_tx as f64 / (1024.0 * 1024.0),
    );

    let header_block = Paragraph::new(Line::from(Span::styled(
        &header,
        Style::default().add_modifier(Modifier::BOLD),
    )))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" efbench - Live Benchmark ")
            .title_alignment(Alignment::Center),
    );
    frame.render_widget(header_block, chunks[0]);

    let chart_area = chunks[1];
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(chart_area);
    let [left, right] = [cols[0], cols[1]];
    let rows_left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(left);
    let rows_right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(right);

    let latency_data: Vec<(f64, f64)> = app
        .latencies
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as f64, v))
        .collect();
    let max_latency = app.latencies.iter().cloned().fold(0.0_f64, f64::max);
    let latency_title =
        format!("Latency (μs)  [cur: {:.0}]", app.current_latency);
    frame.render_widget(
        make_chart(
            &latency_title,
            vec![
                Dataset::default()
                    .name("latency")
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Color::Cyan)
                    .data(&latency_data),
            ],
            "μs",
            max_latency,
        ),
        rows_left[0],
    );

    let tp_data: Vec<(f64, f64)> = app
        .throughputs
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as f64, v))
        .collect();
    let max_tp = app.throughputs.iter().cloned().fold(0.0_f64, f64::max);
    let tp_title =
        format!("Throughput (req/s)  [cur: {}]", app.current_throughput);
    frame.render_widget(
        make_chart(
            &tp_title,
            vec![
                Dataset::default()
                    .name("throughput")
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Color::Green)
                    .data(&tp_data),
            ],
            "req/s",
            max_tp,
        ),
        rows_right[0],
    );

    let rx_data: Vec<(f64, f64)> = app
        .rx_rates
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as f64, v / (1024.0 * 1024.0)))
        .collect();
    let max_rx = rx_data.iter().map(|&(_, v)| v).fold(0.0_f64, f64::max);
    let rx_title = format!(
        "RX (MiB/s)  [cur: {:.1}]",
        app.current_rx as f64 / (1024.0 * 1024.0)
    );
    frame.render_widget(
        make_chart(
            &rx_title,
            vec![
                Dataset::default()
                    .name("RX")
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Color::Yellow)
                    .data(&rx_data),
            ],
            "MiB/s",
            max_rx,
        ),
        rows_left[1],
    );

    let tx_data: Vec<(f64, f64)> = app
        .tx_rates
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as f64, v / (1024.0 * 1024.0)))
        .collect();
    let max_tx = tx_data.iter().map(|&(_, v)| v).fold(0.0_f64, f64::max);
    let tx_title = format!(
        "TX (MiB/s)  [cur: {:.1}]",
        app.current_tx as f64 / (1024.0 * 1024.0)
    );
    frame.render_widget(
        make_chart(
            &tx_title,
            vec![
                Dataset::default()
                    .name("TX")
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Color::Magenta)
                    .data(&tx_data),
            ],
            "MiB/s",
            max_tx,
        ),
        rows_right[1],
    );

    let footer = Paragraph::new(Line::from(" Press 'q' to quit "))
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default());
    frame.render_widget(footer, chunks[2]);
}

pub fn run_tui(
    metrics_rx: mpsc::Receiver<Metrics>,
    app_data: Arc<Mutex<AppData>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut rx = metrics_rx;

    loop {
        // Check for key press (non-blocking)
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press
                    && key.code == KeyCode::Char('q')
                {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                    break;
                }
            }
        }

        // Drain available metrics
        while let Ok(m) = rx.try_recv() {
            if let Ok(mut app) = app_data.lock() {
                app.push_metrics(m);
                app.connected = true;
            }
        }

        // Draw UI
        let app = app_data.lock().unwrap().clone();
        terminal.draw(|f| draw_ui(f, &app))?;

        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
    }

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    Ok(())
}
