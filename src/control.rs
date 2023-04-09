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

use log::{info, warn};
use std::sync::Mutex;
use std::time::Instant;

use crate::config::InverterConfig;
use crate::esp_api::{AreaResponse, API};
use crate::inverter::Inverter;

pub struct State {
    pub response: AreaResponse,
    pub time: Instant,
}

pub async fn poll_esp(api: &API, area_id: &str, state: &Mutex<Option<State>>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        interval.tick().await;
        match api.area(area_id).await {
            Ok(response) => {
                let mut lock = state.lock().unwrap();
                *lock = Some(State {
                    response,
                    time: Instant::now(),
                });
                drop(lock);
                info!("Successfully updated area info from EskomSePush");
            }
            Err(err) => {
                warn!("Failed to update from EskomSePush: {err}");
            }
        }
    }
}

pub async fn control_inverter(
    inverter: Inverter,
    config: &InverterConfig,
    state: &Mutex<Option<State>>,
) {
}
