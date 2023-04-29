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
use std::io::Error;

pub struct Info {
    pub capacity: f64,     // Wh
    pub charge_power: f64, // W
}

#[async_trait]
pub trait Inverter: Send {
    async fn get_info(&mut self) -> Result<Info, Error>;
    async fn set_clock(&mut self, dt: DateTime<Utc>) -> Result<(), Error>;
    async fn set_min_soc(
        &mut self,
        target: f64,
        fallback: f64,
        dt: DateTime<Utc>,
    ) -> Result<(), Error>;
}

/// Wrap another inverter class to turn set methods into nops
pub struct DryrunInverter<T: Inverter> {
    base: T,
}

impl<T: Inverter> DryrunInverter<T> {
    pub fn new(base: T) -> Self {
        Self { base }
    }
}

#[async_trait]
impl<T: Inverter> Inverter for DryrunInverter<T> {
    async fn get_info(&mut self) -> Result<Info, Error> {
        self.base.get_info().await
    }

    async fn set_clock(&mut self, _dt: DateTime<Utc>) -> Result<(), Error> {
        Ok(())
    }

    async fn set_min_soc(
        &mut self,
        _target: f64,
        _fallback: f64,
        _dt: DateTime<Utc>,
    ) -> Result<(), Error> {
        Ok(())
    }
}
