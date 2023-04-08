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

use chrono::naive::NaiveTime;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InverterConfig {
    pub device: String,
    #[serde(default = "id_default")]
    pub id: u8,
    pub min_capacity: u16,
    pub discharge: f64,
    pub charge: f64,
    pub sunset: Option<NaiveTime>,
}

fn id_default() -> u8 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EspConfig {
    pub key: String,
    pub area: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub inverter: InverterConfig,
    pub esp: EspConfig,
}
