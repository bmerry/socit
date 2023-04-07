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

use std::io::Error;

use espd::inverter::Inverter;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let mut inverter = Inverter::new("127.0.0.1:502", 1).await?;
    let programs = inverter.query().await?;
    for program in programs.iter() {
        println!(
            "Time: {}  Power: {}  Capacity: {}",
            program.time, program.power, program.capacity
        );
    }
    Ok(())
}
