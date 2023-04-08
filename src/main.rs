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

use chrono::Local;
use futures::lock::Mutex;
use std::io::Error;
use std::sync::Arc;
use std::time::Duration;

use espd::config::Config;
use espd::esp_api::API;
use espd::inverter::Inverter;

/// Periodically set the inverter's time to match the system time.
async fn time_sync(inverter: Arc<Mutex<Inverter>>) -> Result<(), Error> {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    loop {
        interval.tick().await;
        let now = Local::now();
        inverter.lock().await.set_clock(Local::now()).await?;
        println!("Set clock to {now}");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config: Config = toml::from_str(&std::fs::read_to_string("espd.toml")?)?;

    if false {
        let inverter = Arc::new(Mutex::new(
            Inverter::new(&config.inverter.device, config.inverter.id).await?,
        ));

        let inverter2 = inverter.clone();
        let _handle = tokio::spawn(async move {
            time_sync(inverter2).await?;
            Ok::<(), Error>(())
        });

        let programs = inverter.lock().await.query().await?;
        for program in programs.iter() {
            println!(
                "Time: {}  Power: {}  Capacity: {}",
                program.time, program.power, program.capacity
            );
        }
    }

    let api = API::new(config.esp.key)?;
    let response = api.area("capetown-11-bergvliet").await?;
    println!("{response:?}");

    // handle.await??;
    Ok(())
}
