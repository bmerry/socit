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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::error::Error;

#[derive(Clone, PartialEq, Debug)]
pub struct Update {
    pub time: DateTime<Utc>,
    pub target_soc_low: f64,
    pub target_soc_high: f64,
    pub current_soc: f64,
    pub predicted_pv: f64, // In watts
    pub is_loadshedding: bool,
    pub next_change: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait Monitor: Send {
    async fn update(&mut self, update: Update) -> Result<(), Box<dyn Error>>;
}

pub struct NullMonitor;

#[async_trait]
impl Monitor for NullMonitor {
    async fn update(&mut self, _: Update) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}
