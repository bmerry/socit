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
use chrono::naive::{NaiveDateTime, NaiveTime};
use chrono::{DateTime, Datelike, Duration, DurationRound, Local, Timelike, Utc};
use log::info;
use std::io::Error;
use tokio_modbus::client::Context;
use tokio_modbus::prelude::{Reader, Writer};
use tokio_modbus::slave::Slave;

use super::inverter::{Info, Inverter};

const NUM_PROGRAMS: usize = 6;
const REG_CLOCK: u16 = 22;
const REG_BATTERY_CAPACITY_AH: u16 = 204;
const REG_BATTERY_RESTART_VOLTAGE: u16 = 221;
const REG_GRID_CHARGE_CURRENT: u16 = 230;
const REG_SOC: u16 = 184;
const REG_PROGRAM_TIME: u16 = 250;
const REG_PROGRAM_SOC: u16 = 268;

pub struct SunsynkInverter {
    ctx: Context,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub struct Program {
    pub time: NaiveTime,
    pub soc: u16, // %
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

/// Convert state of charge to u16 and clamp
fn round_soc(soc: f64) -> u16 {
    if soc < 0.0 {
        0
    } else if soc >= 100.0 {
        100
    } else {
        // .round() seems to be broken on Raspberry Pi
        (soc + 0.5) as u16
    }
}

/// Construct programs to load
fn make_programs(target: f64, fallback: f64, now_local: NaiveDateTime) -> [Program; NUM_PROGRAMS] {
    let target = round_soc(target);
    let fallback = round_soc(fallback);
    let mut programs = [Program::default(); NUM_PROGRAMS];
    // The inverter truncates program times to the nearest 5 minutes.
    // Set target in a 20-minute window around the current time.
    let step = Duration::seconds(300);
    programs[0].time = (now_local - step * 2).duration_round(step).unwrap().time();
    programs[1].time = (now_local + step * 2).duration_round(step).unwrap().time();
    // Fill in the rest with 5-minute intervals
    for i in 2..NUM_PROGRAMS {
        programs[i].time = programs[i - 1].time + step;
    }
    // Set target for the current program, fallback_soc for the rest
    programs[0].soc = target;
    for program in programs[1..NUM_PROGRAMS].iter_mut() {
        program.soc = fallback;
    }
    // In some cases the programs will wrap past midnight. Cycle things to keep
    // the start times sorted.
    for i in 1..NUM_PROGRAMS {
        if programs[i].time < programs[i - 1].time {
            programs.rotate_left(i);
            break;
        }
    }
    programs
}

impl SunsynkInverter {
    fn connect(device: &str, modbus_id: u8) -> Context {
        let slave = Slave(modbus_id);
        match device.parse() {
            Ok(socket_addr) => modbus_robust::new_tcp_slave(socket_addr, slave),
            Err(_) => {
                // Not an address. Try it as a device file for serial
                modbus_robust::new_rtu_slave(device, 9600, slave)
            }
        }
    }

    pub fn new(device: &str, modbus_id: u8) -> Self {
        Self {
            ctx: Self::connect(device, modbus_id),
        }
    }

    async fn get_program_field(
        &mut self,
        programs: &mut [Program],
        start: u16,
        apply: impl Fn(&mut Program, u16),
    ) -> Result<(), Error> {
        let values = self
            .ctx
            .read_holding_registers(start, NUM_PROGRAMS as u16)
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
        let mut values = [0u16; NUM_PROGRAMS];
        for (program, value) in programs.iter().zip(values.iter_mut()) {
            *value = get(program);
        }
        self.ctx.write_multiple_registers(start, &values).await?;
        Ok(())
    }

    pub async fn get_programs(&mut self) -> Result<[Program; NUM_PROGRAMS], Error> {
        let mut programs = [Program::default(); NUM_PROGRAMS];
        self.get_program_field(&mut programs, REG_PROGRAM_TIME, |program, x| {
            program.time = decode_time(x).unwrap_or_default();
        })
        .await?;
        self.get_program_field(&mut programs, REG_PROGRAM_SOC, |program, x| {
            program.soc = x;
        })
        .await?;
        Ok(programs)
    }

    pub async fn set_programs(&mut self, programs: &[Program; NUM_PROGRAMS]) -> Result<(), Error> {
        self.set_program_field(programs, REG_PROGRAM_TIME, |program| {
            encode_time(program.time)
        })
        .await?;
        self.set_program_field(programs, REG_PROGRAM_SOC, |program| program.soc)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Inverter for SunsynkInverter {
    async fn get_info(&mut self) -> Result<Info, Error> {
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

    async fn get_soc(&mut self) -> Result<f64, Error> {
        let soc = self.ctx.read_holding_registers(REG_SOC, 1).await?;
        Ok(soc[0] as f64)
    }

    async fn set_clock(&mut self, dt: DateTime<Utc>) -> Result<(), Error> {
        let dt = dt.with_timezone(&Local {}).naive_local();
        let data: [u16; 3] = [
            (((dt.year() - 2000) << 8) as u16) | (dt.month() as u16),
            ((dt.day() << 8) as u16) | (dt.hour() as u16),
            ((dt.minute() << 8) as u16) | (dt.second() as u16),
        ];
        self.ctx.write_multiple_registers(REG_CLOCK, &data).await?;
        Ok(())
    }

    async fn set_min_soc(
        &mut self,
        target: f64,
        fallback: f64,
        dt: DateTime<Utc>,
    ) -> Result<(), Error> {
        let dt = dt.with_timezone(&Local {}).naive_local();
        let programs = make_programs(target, fallback, dt);
        for (i, program) in programs.iter().enumerate() {
            info!(
                "Setting program {} to {}: {}",
                i + 1,
                program.time,
                program.soc
            );
        }
        self.set_programs(&programs).await
    }
}
