/* Copyright 2023 Bruce Merry
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

use log::{error, info};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use socit::config::Config;
use socit::control;
use socit::esp_api::API;
use socit::inverter::Inverter;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let config: Config = toml::from_str(&std::fs::read_to_string("socit.toml")?)?;

    let mut inverter = Inverter::new(&config.inverter.device, config.inverter.id).await?;
    let programs = inverter.get_programs().await?;
    for (i, program) in programs.iter().enumerate() {
        info!("Program {}: {}: {}", i, program.time, program.soc);
    }

    let token = CancellationToken::new();
    let esp_token = token.clone();
    let control_token = token.clone();
    let state = Arc::new(std::sync::Mutex::new(None));
    let state2 = state.clone();
    let api = API::new(config.esp.key)?;
    let esp_handle = tokio::spawn(async move {
        control::poll_esp(
            &api,
            &config.esp.area,
            config.esp.interval,
            &state,
            esp_token,
        )
        .await;
    });
    let control_handle = tokio::spawn(async move {
        // Give poll_esp some time to load the first set of information
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        control::control_inverter(&mut inverter, &config.inverter, &state2, control_token).await;
    });

    match tokio::signal::ctrl_c().await {
        Ok(_) => {}
        Err(err) => {
            error!("Unable to listen for Ctrl-C: {err}");
        }
    }

    token.cancel();
    esp_handle.await?;
    control_handle.await?;
    Ok(())
}
