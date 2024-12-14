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

use async_trait::async_trait;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

pub struct Info {
    pub capacity: f64,     // Wh
    pub charge_power: f64, // W
}

pub struct CoilInfo {
    /// Reading at the CT coil (W) - positive for import from grid
    pub coil: f64,
    /// Reading at the inverter (W) - positive for import from grid
    pub inverter: f64,
    /// Whether the trickle setting applies to the coil
    pub coil_active: bool,
}

#[async_trait]
pub trait Inverter: Send {
    async fn get_info(&mut self) -> Result<Info>;
    async fn get_soc(&mut self) -> Result<f64>;
    async fn set_min_soc(&mut self, target: f64, fallback: f64) -> Result<()>;
    async fn get_coil(&mut self) -> Result<Option<CoilInfo>>;
    async fn set_trickle(&mut self, trickle: f64) -> Result<()>;
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
    async fn get_info(&mut self) -> Result<Info> {
        self.base.get_info().await
    }

    async fn get_soc(&mut self) -> Result<f64> {
        self.base.get_soc().await
    }

    async fn set_min_soc(&mut self, _target: f64, _fallback: f64) -> Result<()> {
        Ok(())
    }

    async fn get_coil(&mut self) -> Result<Option<CoilInfo>> {
        self.base.get_coil().await
    }

    async fn set_trickle(&mut self, _trickle: f64) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use async_trait::async_trait;

    struct TestInverter {
        pub target_soc: f64,
        pub fallback_soc: f64,
        pub soc: f64,
        pub trickle: f64,
        pub inject_error: Option<Error>, // Error returned on next call (one-shot)
    }

    impl TestInverter {
        fn check_inject_error(&mut self) -> Result<()> {
            self.inject_error.take().map_or(Ok(()), |err| Err(err))
        }
    }

    #[async_trait]
    impl Inverter for TestInverter {
        async fn get_info(&mut self) -> Result<Info> {
            self.check_inject_error()?;
            Ok(Info {
                capacity: 5000.0,
                charge_power: 2000.0,
            })
        }

        async fn get_soc(&mut self) -> Result<f64> {
            self.check_inject_error()?;
            Ok(self.soc)
        }

        async fn set_min_soc(&mut self, target: f64, fallback: f64) -> Result<()> {
            self.check_inject_error()?;
            self.target_soc = target;
            self.fallback_soc = fallback;
            Ok(())
        }

        async fn get_coil(&mut self) -> Result<Option<CoilInfo>> {
            self.check_inject_error()?;
            Ok(Some(CoilInfo {
                coil: 450.0,
                inverter: 200.0,
                coil_active: true,
            }))
        }

        async fn set_trickle(&mut self, trickle: f64) -> Result<()> {
            self.check_inject_error()?;
            self.trickle = trickle;
            Ok(())
        }
    }
}
