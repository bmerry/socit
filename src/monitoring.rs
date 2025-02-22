/* Copyright 2023, 2025 Bruce Merry
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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::error::Error;

#[derive(Clone, PartialEq, Debug)]
pub struct SocUpdate {
    pub time: DateTime<Utc>,
    pub target_soc_low: f64,
    pub target_soc_high: f64,
    pub alarm_soc: f64,
    pub current_soc: f64,
    pub predicted_pv: f64, // In watts
    pub is_loadshedding: bool,
    pub next_change: Option<DateTime<Utc>>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct CoilUpdate {
    pub time: DateTime<Utc>,
    pub active: bool,
    pub target: Option<f64>,  // In watts
    pub setting: Option<f64>, // In watts
}

#[async_trait]
pub trait Monitor: Send {
    async fn soc_update(&mut self, update: SocUpdate) -> Result<(), Box<dyn Error>>;
    async fn coil_update(&mut self, update: CoilUpdate) -> Result<(), Box<dyn Error>>;
}

pub struct NullMonitor;

#[async_trait]
impl Monitor for NullMonitor {
    async fn soc_update(&mut self, _: SocUpdate) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    async fn coil_update(&mut self, _: CoilUpdate) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}
