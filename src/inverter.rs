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
use chrono::{Datelike, Timelike};
use std::io::{Error, ErrorKind};
use tokio_modbus::client::Context;
use tokio_modbus::prelude::{Reader, Writer};
use tokio_modbus::slave::Slave;

pub const PROGRAM_BLOCKS: usize = 6;
const REG_CLOCK: u16 = 22;
const REG_SOC: u16 = 184;
const REG_BATTERY_CAPACITY_AH: u16 = 204;
const REG_BATTERY_RESTART_VOLTAGE: u16 = 221;
const REG_GRID_CHARGE_CURRENT: u16 = 230;
const REG_PROGRAM_TIME: u16 = 250;
const REG_PROGRAM_SOC: u16 = 268;

pub struct Inverter {
    ctx: Context,
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct Program {
    pub time: NaiveTime,
    pub soc: u16, // %
}

pub struct Info {
    pub capacity: f64,     // Wh
    pub charge_power: f64, // W
}

/// Decode time from a modbus register.
///
/// If the stored time does not represent a valid time of day, returns None.
fn decode_time(raw: u16) -> Option<NaiveTime> {
    // The time is stored as hours * 100 + minutes.
    let h = raw / 100;
    let m = raw % 100;
    NaiveTime::from_hms_opt(h.into(), m.into(), 0)
}

/// Encode time to store in a modbus register.
///
/// The seconds part of the time is ignored.
fn encode_time(time: NaiveTime) -> u16 {
    (time.hour() * 100 + time.minute()) as u16
}

impl Inverter {
    pub async fn new(device: &str, modbus_id: u8) -> Result<Self, Error> {
        let ctx = match device.parse() {
            Ok(socket_addr) => {
                tokio_modbus::client::tcp::connect_slave(socket_addr, Slave(modbus_id)).await?
            }
            Err(_) => {
                // Not an address. Try it as a device file for serial
                let serial_builder = tokio_serial::new(device, 9600);
                let serial_stream = tokio_serial::SerialStream::open(&serial_builder)?;
                tokio_modbus::client::rtu::connect_slave(serial_stream, Slave(modbus_id)).await?
            }
        };
        Ok(Self { ctx })
    }

    pub async fn get_info(&mut self) -> Result<Info, Error> {
        let capacity_ah = self
            .ctx
            .read_holding_registers(REG_BATTERY_CAPACITY_AH, 1)
            .await?[0] as f64;
        // There are many voltages (low, restart, equalisation, float... this one seems
        // as good as any.
        let voltage = self
            .ctx
            .read_holding_registers(REG_BATTERY_RESTART_VOLTAGE, 1)
            .await?[0] as f64
            * 0.01;
        let charge_current = self
            .ctx
            .read_holding_registers(REG_GRID_CHARGE_CURRENT, 1)
            .await?[0] as f64;
        Ok(Info {
            capacity: capacity_ah * voltage,
            charge_power: charge_current * voltage,
        })
    }

    pub async fn set_clock<T: Datelike + Timelike>(&mut self, dt: &T) -> Result<(), Error> {
        let data: [u16; 3] = [
            (((dt.year() - 2000) << 8) as u16) | (dt.month() as u16),
            ((dt.day() << 8) as u16) | (dt.hour() as u16),
            ((dt.minute() << 8) as u16) | (dt.second() as u16),
        ];
        self.ctx.write_multiple_registers(REG_CLOCK, &data).await?;
        Ok(())
    }

    pub async fn get_soc(&mut self) -> Result<u16, Error> {
        Ok(self.ctx.read_holding_registers(REG_SOC, 1).await?[0])
    }

    async fn get_program_field(
        &mut self,
        programs: &mut [Program],
        start: u16,
        apply: impl Fn(&mut Program, u16),
    ) -> Result<(), Error> {
        let values = self
            .ctx
            .read_holding_registers(start, PROGRAM_BLOCKS as u16)
            .await?;
        for (program, value) in programs.iter_mut().zip(values) {
            apply(program, value);
        }
        Ok(())
    }

    async fn set_program_field(
        &mut self,
        programs: &[Program],
        start: u16,
        get: impl Fn(&Program) -> u16,
    ) -> Result<(), Error> {
        let mut values = [0u16; PROGRAM_BLOCKS];
        for (program, value) in programs.iter().zip(values.iter_mut()) {
            *value = get(program);
        }
        self.ctx.write_multiple_registers(start, &values).await?;
        Ok(())
    }

    pub async fn get_programs(&mut self) -> Result<Vec<Program>, Error> {
        let mut programs = vec![Program::default(); PROGRAM_BLOCKS];
        self.get_program_field(&mut programs, REG_PROGRAM_TIME, |program, x| {
            program.time = decode_time(x).unwrap_or(NaiveTime::default());
        })
        .await?;
        self.get_program_field(&mut programs, REG_PROGRAM_SOC, |program, x| {
            program.soc = x;
        })
        .await?;
        Ok(programs)
    }

    pub async fn set_programs(&mut self, programs: &[Program]) -> Result<(), Error> {
        if programs.len() != PROGRAM_BLOCKS {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "wrong number of programs",
            ));
        }
        self.set_program_field(programs, REG_PROGRAM_TIME, |program| {
            encode_time(program.time)
        })
        .await?;
        self.set_program_field(programs, REG_PROGRAM_SOC, |program| program.soc)
            .await?;
        Ok(())
    }
}
