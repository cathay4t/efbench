// SPDX-License-Identifier: Apache-2.0

use crate::Metrics;

pub fn save_plot(metrics: &[Metrics], output: &str) -> anyhow::Result<()> {
    use plotters::prelude::*;

    let path = format!("{output}.png");
    let root = BitMapBackend::new(&path, (1920, 1080)).into_drawing_area();
    root.fill(&WHITE)?;

    let areas = root.split_evenly((2, 2));

    // --- Latency ---
    let latency_vals: Vec<f64> =
        metrics.iter().map(|m| m.avg_latency_us).collect();
    let max_lat = latency_vals
        .iter()
        .cloned()
        .fold(0.0_f64, f64::max)
        .max(1.0);
    {
        let mut chart = ChartBuilder::on(&areas[0])
            .caption("Latency (μs)", ("sans-serif", 20))
            .margin(10)
            .x_label_area_size(20)
            .y_label_area_size(30)
            .build_cartesian_2d(0..metrics.len(), 0.0..max_lat * 1.1)?;
        chart.configure_mesh().draw()?;
        chart
            .draw_series(LineSeries::new(
                latency_vals.iter().enumerate().map(|(i, &v)| (i, v)),
                &CYAN,
            ))?
            .label("latency")
            .legend(|(x, y)| {
                PathElement::new(vec![(x, y), (x + 20, y)], &CYAN)
            });
        chart.configure_series_labels().draw()?;
    }

    // --- Throughput ---
    let tp_vals: Vec<f64> =
        metrics.iter().map(|m| m.req_count as f64).collect();
    let max_tp = tp_vals.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    {
        let mut chart = ChartBuilder::on(&areas[1])
            .caption("Throughput (req/s)", ("sans-serif", 20))
            .margin(10)
            .x_label_area_size(20)
            .y_label_area_size(30)
            .build_cartesian_2d(0..metrics.len(), 0.0..max_tp * 1.1)?;
        chart.configure_mesh().draw()?;
        chart
            .draw_series(LineSeries::new(
                tp_vals.iter().enumerate().map(|(i, &v)| (i, v)),
                &GREEN,
            ))?
            .label("throughput")
            .legend(|(x, y)| {
                PathElement::new(vec![(x, y), (x + 20, y)], &GREEN)
            });
        chart.configure_series_labels().draw()?;
    }

    // --- RX ---
    let rx_vals: Vec<f64> = metrics
        .iter()
        .map(|m| m.rx_bytes as f64 / (1024.0 * 1024.0))
        .collect();
    let max_rx = rx_vals.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    {
        let mut chart = ChartBuilder::on(&areas[2])
            .caption("RX (MiB/s)", ("sans-serif", 20))
            .margin(10)
            .x_label_area_size(20)
            .y_label_area_size(30)
            .build_cartesian_2d(0..metrics.len(), 0.0..max_rx * 1.1)?;
        chart.configure_mesh().draw()?;
        chart
            .draw_series(LineSeries::new(
                rx_vals.iter().enumerate().map(|(i, &v)| (i, v)),
                &YELLOW,
            ))?
            .label("RX")
            .legend(|(x, y)| {
                PathElement::new(vec![(x, y), (x + 20, y)], &YELLOW)
            });
        chart.configure_series_labels().draw()?;
    }

    // --- TX ---
    let tx_vals: Vec<f64> = metrics
        .iter()
        .map(|m| m.tx_bytes as f64 / (1024.0 * 1024.0))
        .collect();
    let max_tx = tx_vals.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    {
        let mut chart = ChartBuilder::on(&areas[3])
            .caption("TX (MiB/s)", ("sans-serif", 20))
            .margin(10)
            .x_label_area_size(20)
            .y_label_area_size(30)
            .build_cartesian_2d(0..metrics.len(), 0.0..max_tx * 1.1)?;
        chart.configure_mesh().draw()?;
        chart
            .draw_series(LineSeries::new(
                tx_vals.iter().enumerate().map(|(i, &v)| (i, v)),
                &MAGENTA,
            ))?
            .label("TX")
            .legend(|(x, y)| {
                PathElement::new(vec![(x, y), (x + 20, y)], &MAGENTA)
            });
        chart.configure_series_labels().draw()?;
    }

    root.present()?;
    println!("Plot saved to {path}");
    Ok(())
}
