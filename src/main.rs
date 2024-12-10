/* Copyright 2023-2024 Bruce Merry
 *
 * This program is free software: you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation, either version 3 of the License, or (at your option)
 * any later version.
 *
 * This program is distributed in the hope that it will be useful, but WITHOUT
 * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
 * FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
 * more details.
 *
 * You should have received a copy of the GNU General Public License along
 * with this program. If not, see <https://www.gnu.org/licenses/>.
 */

use clap::Parser;
use log::info;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

use socit::config::Config;
use socit::control;
use socit::esp_api::API;
use socit::influxdb2::Influxdb2Monitor;
use socit::inverter::{DryrunInverter, Inverter};
use socit::monitoring::{Monitor, NullMonitor};
use socit::sunsynk::SunsynkInverter;

#[derive(Parser)]
#[clap(author, version)]
struct Args {
    #[clap()]
    config_file: PathBuf,
}

#[cfg(unix)]
async fn wait_shutdown() -> std::io::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = sigint.recv() => {},
        _ = sigterm.recv() => {},
    };
    Ok(())
}

#[cfg(not(unix))]
async fn wait_shutdown() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Args::parse();
    let config: Config = toml::from_str(&std::fs::read_to_string(args.config_file)?)?;
    let esp_timeout = chrono::Duration::from_std(config.esp.timeout)?;

    let mut inverter = SunsynkInverter::new(&config.inverter.device, config.inverter.id);
    if let Ok(programs) = inverter.get_programs().await {
        for (i, program) in programs.iter().enumerate() {
            info!("Program {}: {}: {}", i, program.time, program.soc);
        }
    }

    let mut inverter: Box<dyn Inverter> = if config.inverter.dry_run {
        Box::new(DryrunInverter::new(inverter))
    } else {
        Box::new(inverter)
    };

    let token = CancellationToken::new();
    let esp_token = token.clone();
    let control_token = token.clone();
    let state = Arc::new(Mutex::new(None));
    let state2 = state.clone();
    /* TODO: see if there is a nice way to avoid cloning (std::mem::take
     * requires making config mutable).
     */
    let api = API::new(config.esp.key.clone())?;
    let area = config.esp.area.clone();
    let esp_handle = tokio::spawn(async move {
        control::poll_esp(&api, &area, config.esp.interval, &state, esp_token).await;
    });
    let mut monitor: Box<dyn Monitor> = match &config.influxdb2 {
        Some(conf) => Box::new(Influxdb2Monitor::new(conf).await),
        None => Box::new(NullMonitor {}),
    };
    let control_handle = tokio::spawn(async move {
        // Give poll_esp some time to load the first set of information
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        control::control_inverter(
            inverter.as_mut(),
            &config,
            &mut *monitor,
            &state2,
            esp_timeout,
            control_token,
        )
        .await;
    });

    wait_shutdown().await?;
    token.cancel();
    esp_handle.await?;
    control_handle.await?;
    Ok(())
}
