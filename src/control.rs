/* Copyright 2023-2025 Bruce Merry
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
use chrono::{DateTime, Duration, Utc};
use futures::StreamExt;
use log::{error, info, warn};
use radians::Deg64;
use std::cmp::min;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;
use tokio::time::MissedTickBehavior;
use tokio_stream::StreamMap;
use tokio_util::sync::CancellationToken;

use crate::config::{CoilConfig, Config, InverterConfig, PanelConfig};
use crate::esp_api::{AreaResponse, API};
use crate::inverter::{Info, Inverter, Result};
use crate::monitoring::{CoilUpdate, Monitor, SocUpdate};
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

/// Number of (non-integer) hours in a duration
fn duration_hours(duration: Duration) -> f64 {
    (duration.num_milliseconds() as f64) / 3600000.0
}

fn panels_power(panels: &[PanelConfig], time: DateTime<Utc>) -> f64 {
    let mut power = 0.0;
    for panels in panels.iter() {
        power += panels.power
            * solar_fraction(
                Deg64::new(panels.latitude),
                Deg64::new(panels.longitude),
                Deg64::new(90.0 - panels.tilt),
                Deg64::new(panels.azimuth),
                &time,
            );
    }
    power
}

/// What to simulate when no load-shedding and not enough solar
enum SimMode {
    /// Power drains from battery
    Drain,
    /// Battery level held steady
    Hold,
    /// Charge battery as fast as possible
    Charge,
}

fn target_soc_helper(
    config: &InverterConfig,
    state: &State,
    info: &Info,
    now: DateTime<Utc>,
    mode: SimMode,
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
        let mut power = panels_power(&config.panels, t + step / 2);
        if let Some(charge_power) = config.charge_power {
            power = power.min(charge_power);
        }
        power -= config.min_discharge_power;
        if have_grid {
            power = match mode {
                SimMode::Drain => power,
                SimMode::Hold => power.max(0.0),
                SimMode::Charge => config.charge_power.unwrap_or(power),
            };
        }
        base_wh += power * step_h;
        t += step;

        floor = floor.max(base_wh - depth);
        observe(base_wh.max(floor), t);
    }

    let extra = -worst / info.capacity * 100.0;
    let target = config.min_soc + extra;
    let target = target.clamp(0.0, 100.0);
    (target, worst_time)
}

fn target_socs(
    config: &InverterConfig,
    state: Option<&State>,
    info: &Info,
    now: DateTime<Utc>,
) -> (f64, f64, f64) {
    match state {
        None => (config.fallback_soc, config.fallback_soc, config.min_soc),
        Some(state) => {
            for event in state.response.events.iter() {
                info!("Load-shedding from {} to {}", event.start, event.end);
            }
            let (target_high, _) = target_soc_helper(config, state, info, now, SimMode::Drain);
            let (target_low, _) = target_soc_helper(config, state, info, now, SimMode::Hold);
            let (alarm, _) = target_soc_helper(config, state, info, now, SimMode::Charge);
            (target_low, target_high, alarm)
        }
    }
}

async fn update_soc(
    inverter: &mut dyn Inverter,
    config: &InverterConfig,
    monitor: &mut dyn Monitor,
    state: &Mutex<Option<State>>,
    esp_timeout: Duration,
) -> Result<()> {
    let now = Utc::now();
    let info = inverter.get_info().await?;
    let current_soc = inverter.get_soc().await?;
    let target;
    let update;

    {
        let guard = &state.lock().unwrap();
        let state = filter_state(guard, now - esp_timeout);
        let est_start = Instant::now();
        let (target_soc_low, target_soc_high, alarm_soc) = target_socs(config, state, &info, now);
        info!(
            "Target SoC range is {:.2} - {:.2} (alarm at {:.2}), computed in {:.3} s",
            target_soc_low,
            target_soc_high,
            alarm_soc,
            est_start.elapsed().as_secs_f64()
        );
        target = current_soc.min(target_soc_high).max(target_soc_low);

        let mut is_loadshedding = false;
        let mut next_change = None;
        if let Some(state) = state {
            for event in state.response.events.iter() {
                if now >= event.start && now < event.end {
                    is_loadshedding = true;
                    next_change = Some(event.end);
                    break;
                } else if now < event.start {
                    next_change = Some(next_change.map_or(event.start, |t| min(t, event.start)));
                }
            }
        }

        update = SocUpdate {
            time: now,
            target_soc_low,
            target_soc_high,
            alarm_soc,
            current_soc,
            predicted_pv: panels_power(&config.panels, now),
            is_loadshedding,
            next_change,
        };
    }

    inverter.set_min_soc(target, config.fallback_soc).await?;
    if let Err(err) = monitor.soc_update(update).await {
        warn!("Failed to update monitoring: {err}");
    }

    Ok(())
}

#[async_trait]
trait Controller: Send + Unpin {
    fn interval(&self) -> std::time::Duration;
    async fn update(&mut self, inverter: &mut dyn Inverter, monitor: &mut dyn Monitor);
    async fn shutdown(&mut self, inverter: &mut dyn Inverter);
}

struct SocController<'a> {
    config: &'a InverterConfig,
    state: &'a Mutex<Option<State>>,
    esp_timeout: Duration,
}

impl<'a> SocController<'a> {
    fn new(
        config: &'a InverterConfig,
        state: &'a Mutex<Option<State>>,
        esp_timeout: Duration,
    ) -> Self {
        Self {
            config,
            state,
            esp_timeout,
        }
    }
}

#[async_trait]
impl Controller for SocController<'_> {
    fn interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(60)
    }

    async fn update(&mut self, inverter: &mut dyn Inverter, monitor: &mut dyn Monitor) {
        if let Err(err) =
            update_soc(inverter, self.config, monitor, self.state, self.esp_timeout).await
        {
            warn!("Failed to update inverter: {err}");
        }
    }

    async fn shutdown(&mut self, inverter: &mut dyn Inverter) {
        info!(
            "Shutting down, setting minimum SoC to {}",
            self.config.fallback_soc
        );
        match inverter
            .set_min_soc(self.config.fallback_soc, self.config.fallback_soc)
            .await
        {
            Ok(_) => {}
            Err(err) => {
                error!("Failed to set minimum SoC: {err}");
            }
        }
    }
}

struct CoilController<'a> {
    history: VecDeque<Option<f64>>,
    config: &'a CoilConfig,
    last_setting: Option<f64>,
}

impl<'a> CoilController<'a> {
    const CAPACITY: usize = 11;

    fn new(config: &'a CoilConfig) -> Self {
        Self {
            history: VecDeque::with_capacity(Self::CAPACITY),
            config,
            last_setting: None,
        }
    }

    async fn update_fallible(
        &mut self,
        inverter: &mut dyn Inverter,
        monitor: &mut dyn Monitor,
    ) -> Result<()> {
        let info = inverter.get_coil().await?;
        let mut target = None;
        if let Some(value) = &info {
            let ne = value.coil - value.inverter;
            if ne <= self.config.power_threshold {
                // It's fake power from misreading coil
                target = Some(ne + self.config.trickle);
            }
        }
        if self.history.len() == Self::CAPACITY {
            self.history.pop_front();
        }
        self.history.push_back(target);
        if self.history.len() != Self::CAPACITY {
            return Ok(());
        }
        // Compute the sum if all elements are not None
        let sum = self.history.iter().cloned().sum::<Option<f64>>();
        let mean = sum.map(|x| x / (self.history.len() as f64));
        let coil_active = info.map_or(false, |x| x.coil_active);
        if coil_active {
            if let Some(target) = mean {
                if self
                    .last_setting
                    .map_or(true, |x| (x - target).abs() >= 10.0)
                {
                    info!("Setting trickle to {target}.");
                    inverter.set_trickle(target).await?;
                    self.last_setting = Some(target);
                } else {
                    info!("Ideal trickle setting is {target}, but not setting due to hysteresis.");
                }
            } else {
                info!("Not adjusting trickle because there is no target information.")
            }
        } else {
            info!("Not adjusting trickle because coil is not active.");
        }
        let update = CoilUpdate {
            time: Utc::now(),
            active: coil_active,
            target: mean,
            setting: self.last_setting,
        };
        if let Err(err) = monitor.coil_update(update).await {
            warn!("Failed to update monitoring: {err}");
        }
        Ok(())
    }
}

#[async_trait]
impl Controller for CoilController<'_> {
    fn interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(10)
    }

    async fn update(&mut self, inverter: &mut dyn Inverter, monitor: &mut dyn Monitor) {
        match self.update_fallible(inverter, monitor).await {
            Ok(_) => {}
            Err(err) => {
                error!("Failed to update CT coil: {err}");
            }
        }
    }

    async fn shutdown(&mut self, _inverter: &mut dyn Inverter) {}
}

pub async fn control_inverter(
    inverter: &mut dyn Inverter,
    config: &Config,
    monitor: &mut dyn Monitor,
    state: &Mutex<Option<State>>,
    esp_timeout: Duration,
    token: CancellationToken,
) {
    let mut controllers: Vec<Box<dyn Controller>> = Vec::new();
    controllers.push(Box::new(SocController::new(
        &config.inverter,
        state,
        esp_timeout,
    )));
    if let Some(coil_config) = &config.coil {
        controllers.push(Box::new(CoilController::new(coil_config)));
    }
    let mut stream = StreamMap::new();
    for (i, controller) in controllers.iter().enumerate() {
        let mut interval = tokio::time::interval(controller.interval());
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        stream.insert(i, tokio_stream::wrappers::IntervalStream::new(interval));
    }

    loop {
        tokio::select! {
            Some((idx, _)) = stream.next() => { controllers[idx].update(inverter, monitor).await; }
            _ = token.cancelled() => { break; }
        }
    }

    for controller in controllers.iter_mut() {
        controller.shutdown(inverter).await;
    }
}
