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
use std::io::Error;
use std::sync::Mutex;
use std::time::Instant;
use tokio::time::MissedTickBehavior;

use crate::config::InverterConfig;
use crate::esp_api::{AreaResponse, API};
use crate::inverter::{Info, Inverter, Program, PROGRAM_BLOCKS};
use crate::sun::solar_fraction;

pub struct State {
    pub response: AreaResponse,
    pub time: DateTime<Utc>,
}

pub async fn poll_esp(
    api: &API,
    area_id: &str,
    interval: std::time::Duration,
    state: &Mutex<Option<State>>,
) {
    let mut interval = tokio::time::interval(interval);
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

fn filter_state<'a>(state: &'a Option<State>, now: &DateTime<Utc>) -> Option<&'a State> {
    // TODO: make timeout configurable
    state
        .as_ref()
        .filter(|state| *now - state.time <= Duration::seconds(4 * 3600))
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
        // .round() seems to be broken on Raspberry Pi
        (soc + 0.5) as u16
    }
}

fn target_soc(
    config: &InverterConfig,
    state: &Mutex<Option<State>>,
    info: &Info,
    now: &DateTime<Utc>,
) -> u16 {
    let guard = &state.lock().unwrap();
    match filter_state(guard, now) {
        None => config.fallback_soc,
        Some(state) => {
            let mut base_wh = 0.0;
            let mut worst = 0.0_f64;
            let mut worst_time = *now;
            /* Project battery level forward for 24 hours, using optimistic
             * assumptions about solar PV and consumption. Whenever the
             * current point falls into load-shedding, check that there will
             * be enough to get to the end with pessimistic assumptions.
             */
            let step = Duration::seconds(60);
            let step_h = duration_hours(step);
            let goal = *now + Duration::seconds(86400);
            let mut t = *now;
            for event in state.response.events.iter() {
                info!("Load-shedding from {} to {}", event.start, event.end);
            }
            while t < goal {
                for event in state.response.events.iter() {
                    if t >= event.start && t < event.end {
                        let end_wh =
                            base_wh - config.max_discharge_power * duration_hours(event.end - t);
                        if end_wh < worst {
                            worst = end_wh;
                            worst_time = t;
                        }
                    }
                }
                let mut power = -config.min_discharge_power;
                for panels in config.panels.iter() {
                    power += panels.power
                        * solar_fraction(
                            panels.latitude.to_radians(),
                            panels.longitude.to_radians(),
                            (90.0 - panels.tilt).to_radians(),
                            panels.azimuth.to_radians(),
                            &(t + step / 2),
                        );
                }
                base_wh += power * step_h;
                t += step;

                if base_wh < worst {
                    worst = base_wh;
                    worst_time = t;
                }
            }
            info!(
                "Maximum decrease is {} Wh at {}",
                -worst,
                to_local(worst_time)
            );
            let extra = -worst / info.capacity * 100.0;
            round_soc((config.min_soc as f64) + extra)
        }
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
    let info = inverter.get_info().await?;

    let est_start = Instant::now();
    let target = target_soc(config, state, &info, &now);
    info!(
        "Target SoC is {}, computed in {} s",
        target,
        est_start.elapsed().as_secs_f64()
    );

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
    // Set target for the current program, fallback_soc for the rest
    programs[0].soc = target;
    for program in programs[1..PROGRAM_BLOCKS].iter_mut() {
        program.soc = config.fallback_soc;
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
            program.soc
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
