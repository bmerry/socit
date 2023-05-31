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

use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelConfig {
    pub latitude: f64,
    pub longitude: f64,
    pub tilt: f64,
    pub azimuth: f64,
    pub power: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InverterConfig {
    pub device: String,
    #[serde(default = "id_default")]
    pub id: u8,
    // TODO: validation of range
    pub min_soc: f64,
    pub fallback_soc: f64,
    pub min_discharge_power: f64,
    pub max_discharge_power: f64,
    #[serde(default)]
    pub charge_power: Option<f64>,
    #[serde(default = "dry_run_default")]
    pub dry_run: bool,
    #[serde(default)]
    pub panels: Vec<PanelConfig>,
}

fn id_default() -> u8 {
    1
}

fn dry_run_default() -> bool {
    false
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EspConfig {
    pub key: String,
    pub area: String,
    #[serde(default = "interval_default", with = "humantime_serde")]
    pub interval: Duration,
    #[serde(default = "timeout_default", with = "humantime_serde")]
    pub timeout: Duration,
}

fn interval_default() -> Duration {
    // Default to 40 minutes
    Duration::from_secs(40 * 60)
}

fn timeout_default() -> Duration {
    // Default to 4 hours
    Duration::from_secs(4 * 60 * 60)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Influxdb2Config {
    #[serde(default = "default_host")]
    pub host: String,
    pub org: String,
    pub token: String,
    pub bucket: String,
}

fn default_host() -> String {
    "http://localhost:8086".to_string()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub inverter: InverterConfig,
    pub esp: EspConfig,
    pub influxdb2: Option<Influxdb2Config>,
}
