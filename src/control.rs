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
use log::{error, info, warn};
use std::io::Error;
use std::sync::Mutex;
use std::time::Instant;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::config::InverterConfig;
use crate::esp_api::{AreaResponse, API};
use crate::inverter::{Info, Inverter};
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
    token: CancellationToken,
) {
    let mut interval = tokio::time::interval(interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = interval.tick() => {},
            _ = token.cancelled() => { break; }
        }
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

fn filter_state(state: &Option<State>, min_time: DateTime<Utc>) -> Option<&State> {
    state.as_ref().filter(|state| state.time >= min_time)
}

// Number of (non-integer) hours in a duration
fn duration_hours(duration: Duration) -> f64 {
    (duration.num_milliseconds() as f64) / 3600000.0
}

fn to_local<Tz: TimeZone>(dt: DateTime<Tz>) -> NaiveDateTime {
    dt.with_timezone(&Local {}).naive_local()
}

fn target_soc(
    config: &InverterConfig,
    state: &Mutex<Option<State>>,
    info: &Info,
    now: DateTime<Utc>,
    esp_timeout: Duration,
) -> f64 {
    let guard = &state.lock().unwrap();
    match filter_state(guard, now - esp_timeout) {
        None => config.fallback_soc,
        Some(state) => {
            let step = Duration::seconds(60);
            let step_h = duration_hours(step);
            let depth = info.capacity - config.min_soc * 0.01 * info.capacity;

            let mut base_wh = 0.0;
            let mut worst = 0.0_f64;
            let mut floor = -depth;
            let mut worst_time = now;
            /* Project battery level forward for 24 hours, using optimistic
             * assumptions about solar PV and consumption. Whenever the
             * current point falls into load-shedding, check that there will
             * be enough to get to the end with pessimistic assumptions.
             */
            let goal = now + Duration::seconds(86400);
            let mut t = now;
            for event in state.response.events.iter() {
                info!("Load-shedding from {} to {}", event.start, event.end);
            }
            let mut observe = |wh, t| {
                if wh < worst {
                    worst = wh;
                    worst_time = t;
                }
            };
            while t < goal {
                for event in state.response.events.iter() {
                    if t >= event.start && t < event.end {
                        let end_wh =
                            base_wh - config.max_discharge_power * duration_hours(event.end - t);
                        observe(end_wh.max(floor), t);
                    }
                }
                let mut power = 0.0;
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
                if let Some(charge_power) = config.charge_power {
                    power = power.min(charge_power);
                }
                power -= config.min_discharge_power;
                base_wh += power * step_h;
                t += step;

                floor = floor.max(base_wh - depth);
                observe(base_wh.max(floor), t);
            }
            info!(
                "Maximum decrease is {} Wh at {}",
                -worst,
                to_local(worst_time)
                    .duration_round(Duration::seconds(1))
                    .unwrap()
            );
            let extra = -worst / info.capacity * 100.0;
            let target = config.min_soc + extra;
            target.max(0.0).min(100.0) // clamp to 0-100
        }
    }
}

async fn update_inverter(
    inverter: &mut Box<dyn Inverter>,
    config: &InverterConfig,
    state: &Mutex<Option<State>>,
    esp_timeout: Duration,
) -> Result<(), Error> {
    let now = Utc::now();
    info!("Setting inverter time to {now}");
    inverter.set_clock(now).await?;
    let info = inverter.get_info().await?;

    let est_start = Instant::now();
    let target = target_soc(config, state, &info, now, esp_timeout);
    info!(
        "Target SoC is {}, computed in {} s",
        target,
        est_start.elapsed().as_secs_f64()
    );

    inverter
        .set_min_soc(target, config.fallback_soc, now)
        .await?;
    Ok(())
}

pub async fn control_inverter(
    inverter: &mut Box<dyn Inverter>,
    config: &InverterConfig,
    state: &Mutex<Option<State>>,
    esp_timeout: Duration,
    token: CancellationToken,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = interval.tick() => {},
            _ = token.cancelled() => { break; }
        }
        if let Err(err) = update_inverter(inverter, config, state, esp_timeout).await {
            warn!("Failed to update inverter: {err}");
        }
    }

    info!(
        "Shutting down, setting minimum SoC to {}",
        config.fallback_soc
    );
    match inverter
        .set_min_soc(config.fallback_soc, config.fallback_soc, Utc::now())
        .await
    {
        Ok(_) => {}
        Err(err) => {
            error!("Failed to set minimum SoC: {err}");
        }
    }
}
