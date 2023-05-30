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

use chrono::{DateTime, Duration, Utc};
use log::{error, info, warn};
use radians::Deg64;
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

fn target_soc_helper(
    config: &InverterConfig,
    state: &State,
    info: &Info,
    now: DateTime<Utc>,
    clamp: bool,
) -> (f64, DateTime<Utc>) {
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
     *
     * If clamp is true, assume that the battery level will not decline
     * except during load shedding (maintained with grid power).
     */
    let goal = now + Duration::seconds(86400);
    let mut t = now;
    let mut observe = |wh, t| {
        if wh < worst {
            worst = wh;
            worst_time = t;
        }
    };
    while t < goal {
        let mut have_grid = true;
        for event in state.response.events.iter() {
            if t >= event.start && t < event.end {
                have_grid = false;
                let end_wh = base_wh - config.max_discharge_power * duration_hours(event.end - t);
                observe(end_wh.max(floor), t);
            }
        }
        let mut power = 0.0;
        for panels in config.panels.iter() {
            power += panels.power
                * solar_fraction(
                    Deg64::new(panels.latitude),
                    Deg64::new(panels.longitude),
                    Deg64::new(90.0 - panels.tilt),
                    Deg64::new(panels.azimuth),
                    &(t + step / 2),
                );
        }
        if let Some(charge_power) = config.charge_power {
            power = power.min(charge_power);
        }
        power -= config.min_discharge_power;
        if clamp && have_grid && power < 0.0 {
            power = 0.0;
        }
        base_wh += power * step_h;
        t += step;

        floor = floor.max(base_wh - depth);
        observe(base_wh.max(floor), t);
    }

    let extra = -worst / info.capacity * 100.0;
    let target = config.min_soc + extra;
    let target = target.max(0.0).min(100.0); // clamp to 0-100
    (target, worst_time)
}

fn target_socs(
    config: &InverterConfig,
    state: &Mutex<Option<State>>,
    info: &Info,
    now: DateTime<Utc>,
    esp_timeout: Duration,
) -> (f64, f64) {
    let guard = &state.lock().unwrap();
    match filter_state(guard, now - esp_timeout) {
        None => (config.fallback_soc, config.fallback_soc),
        Some(state) => {
            for event in state.response.events.iter() {
                info!("Load-shedding from {} to {}", event.start, event.end);
            }
            let (target_high, _) = target_soc_helper(config, state, info, now, false);
            let (target_low, _) = target_soc_helper(config, state, info, now, true);
            (target_low, target_high)
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
    let (target_low, target_high) = target_socs(config, state, &info, now, esp_timeout);
    info!(
        "Target SoC range is {:.2} - {:.2}, computed in {:.3} s",
        target_low,
        target_high,
        est_start.elapsed().as_secs_f64()
    );

    let current_soc = inverter.get_soc().await?;
    let target = current_soc.min(target_high).max(target_low);
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
