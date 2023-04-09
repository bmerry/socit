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

use chrono::naive::NaiveDateTime;
use chrono::{DateTime, Duration, DurationRound, Local, TimeZone, Utc};
use log::{info, warn};
use std::cmp::max;
use std::io::Error;
use std::sync::Mutex;
use tokio::time::MissedTickBehavior;

use crate::config::InverterConfig;
use crate::esp_api::{AreaResponse, API};
use crate::inverter::{Inverter, Program, PROGRAM_BLOCKS};

pub struct State {
    pub response: AreaResponse,
    pub time: DateTime<Utc>,
}

enum LoadShedding {
    Never,
    Unknown,
    Soon(DateTime<Utc>, DateTime<Utc>),
}

pub async fn poll_esp(api: &API, area_id: &str, state: &Mutex<Option<State>>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match api.area(area_id).await {
            Ok(response) => {
                let mut lock = state.lock().unwrap();
                *lock = Some(State {
                    response,
                    time: Utc::now(),
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

fn next_load_shedding(state: &Option<State>, now: &DateTime<Utc>) -> LoadShedding {
    match state {
        None => LoadShedding::Unknown,
        Some(state) => {
            // TODO: make timeout configurable
            if *now - state.time > Duration::seconds(4 * 3600) {
                LoadShedding::Unknown // Load-shedding information is too stale
            } else {
                // Get the earliest event that isn't in the past
                match state
                    .response
                    .events
                    .iter()
                    .filter(|event| &event.end > now)
                    .min_by_key(|event| event.start)
                {
                    Some(event) => LoadShedding::Soon(*max(&event.start, now), event.end),
                    None => LoadShedding::Never,
                }
            }
        }
    }
}

// Number of (non-integer) hours in a duration
fn duration_hours(duration: Duration) -> f64 {
    (duration.num_milliseconds() as f64) / 3600000.0
}

fn to_local<Tz: TimeZone>(dt: DateTime<Tz>) -> NaiveDateTime {
    dt.with_timezone(&Local {}).naive_local()
}

// Convert state of charge to u16 and clamp
fn round_soc(soc: f64) -> u16 {
    if soc < 0.0 {
        0
    } else if soc >= 100.0 {
        100
    } else {
        soc.round() as u16
    }
}

async fn update_inverter(
    inverter: &mut Inverter,
    config: &InverterConfig,
    state: &Mutex<Option<State>>,
) -> Result<(), Error> {
    let now = Utc::now();
    let now_local = to_local(now);
    info!("Setting inverter time to {now_local}");
    if !config.dry_run {
        inverter.set_clock(&now_local).await?;
    }

    let target_soc: u16 = match next_load_shedding(&state.lock().unwrap(), &now) {
        LoadShedding::Soon(start, end) => {
            // _wh suffix indices _wh; _soc indicates percentage
            let end_wh = (config.min_soc as f64) * 0.01 * config.capacity;
            let length = duration_hours(end - start);
            // TODO: add in solar here
            let start_wh = end_wh + config.charge * length;
            let now_wh = start_wh - config.discharge * duration_hours(start - now);
            let now_soc = now_wh / config.capacity;
            max(config.min_soc, round_soc(now_soc))
        }
        LoadShedding::Never => config.min_soc,
        LoadShedding::Unknown => config.fallback_soc,
    };

    let mut programs = vec![Program::default(); PROGRAM_BLOCKS];
    // The inverter truncates program times to the nearest 5 minutes. Let target_soc
    // in a 20-minute window around the current time.
    let step = Duration::seconds(300);
    programs[0].time = (now_local - step * 2).duration_round(step).unwrap().time();
    programs[1].time = (now_local + step * 2).duration_round(step).unwrap().time();
    // Fill in the rest with 5-minute intervals
    for i in 2..PROGRAM_BLOCKS {
        programs[i].time = programs[i - 1].time + step;
    }
    // Set target_soc for the current program, fallback_soc for the rest
    programs[0].capacity = target_soc;
    for program in programs[1..PROGRAM_BLOCKS].iter_mut() {
        program.capacity = config.fallback_soc;
    }
    // In some cases the programs will wrap past midnight. Cycle things to keep
    // the start times sorted.
    for i in 1..PROGRAM_BLOCKS {
        if programs[i].time < programs[i - 1].time {
            programs.rotate_left(i);
            break;
        }
    }

    for (i, program) in programs.iter().enumerate() {
        info!(
            "Setting program {} to {}: {}",
            i + 1,
            program.time,
            program.capacity
        );
    }
    if !config.dry_run {
        inverter.set_programs(&programs).await?;
    }

    Ok(())
}

pub async fn control_inverter(
    inverter: &mut Inverter,
    config: &InverterConfig,
    state: &Mutex<Option<State>>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Err(err) = update_inverter(inverter, config, state).await {
            warn!("Failed to update inverter: {err}");
        }
    }
}
