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

use chrono::naive::NaiveDate;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone, Debug, Deserialize)]
pub struct Event {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub note: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Info {
    pub name: String,
    pub region: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ScheduleDay {
    pub date: NaiveDate,
    pub name: String,
    pub stages: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Schedule {
    pub days: Vec<ScheduleDay>,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AreaResponse {
    pub events: Vec<Event>,
    pub info: Info,
    pub schedule: Schedule,
}

pub struct API {
    key: String,
    client: Client,
}

impl API {
    pub fn new(key: impl Into<String>) -> reqwest::Result<Self> {
        Ok(Self {
            key: key.into(),
            client: reqwest::ClientBuilder::new()
                .timeout(Duration::from_secs(10))
                .build()?,
        })
    }

    pub async fn area(&self, id: &str) -> reqwest::Result<AreaResponse> {
        self.client
            .get("https://developer.sepush.co.za/business/2.0/area")
            .query(&[("id", id), ("test", "current")])
            .header("Token", &self.key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
}
