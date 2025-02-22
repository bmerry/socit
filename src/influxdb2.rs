/* Copyright 2023, 2025 Bruce Merry
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
use influxdb2::api::write::TimestampPrecision;
use influxdb2::models::health::Status;
use influxdb2::models::DataPoint;
use influxdb2::Client;
use log::{info, warn};
use std::error::Error;

use crate::config::Influxdb2Config;
use crate::monitoring::{CoilUpdate, Monitor, SocUpdate};

pub struct Influxdb2Monitor {
    client: Client,
    bucket: String,
}

impl Influxdb2Monitor {
    pub async fn new(config: &Influxdb2Config) -> Self {
        let client = Client::new(&config.host, &config.org, &config.token);
        match client.health().await {
            Ok(health_check) => {
                if health_check.status == Status::Fail {
                    match health_check.message {
                        Some(ref message) => {
                            warn!("Influxdb server is unhealthy: {}", message);
                        }
                        None => {
                            warn!("Influxdb server is unhealthy");
                        }
                    }
                } else {
                    info!(
                        "Successfully connected to Influxdb server at {}",
                        &config.host
                    );
                }
            }
            Err(err) => {
                warn!("Could not connect to Influxdb server: {}", err);
            }
        }
        Self {
            client,
            bucket: config.bucket.to_owned(),
        }
    }
}

#[async_trait]
impl Monitor for Influxdb2Monitor {
    async fn soc_update(&mut self, update: SocUpdate) -> Result<(), Box<dyn Error>> {
        let mut builder = DataPoint::builder("socit")
            .timestamp(update.time.timestamp())
            .field("target_soc_low", update.target_soc_low)
            .field("target_soc_high", update.target_soc_high)
            .field("alarm_soc", update.alarm_soc)
            .field("current_soc", update.current_soc)
            .field("predicted_pv", update.predicted_pv)
            .field("is_loadshedding", update.is_loadshedding);
        if let Some(next_change) = update.next_change {
            builder = builder.field(
                "next_change_seconds",
                (next_change - update.time).num_milliseconds() as f64 * 1e-3,
            );
        }
        let point = builder.build().unwrap();
        let strm = futures::stream::once(async { point });
        self.client
            .write_with_precision(&self.bucket, strm, TimestampPrecision::Seconds)
            .await?;
        Ok(())
    }

    async fn coil_update(&mut self, update: CoilUpdate) -> Result<(), Box<dyn Error>> {
        let mut builder = DataPoint::builder("socit-coil")
            .timestamp(update.time.timestamp())
            .field("active", update.active);
        if let Some(target) = update.target {
            builder = builder.field("target", target);
        }
        if let Some(setting) = update.setting {
            builder = builder.field("setting", setting);
        }
        let point = builder.build().unwrap();
        let strm = futures::stream::once(async { point });
        self.client
            .write_with_precision(&self.bucket, strm, TimestampPrecision::Seconds)
            .await?;
        Ok(())
    }
}
