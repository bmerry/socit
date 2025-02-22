# State-of-charge control

This package actively controls the minimum state of charge (battery level) to
adapt to load shedding and to reduce unnecessary cycling of the battery. It
integrates with the [EskomSePush API](https://eskomsepush.gumroad.com/l/api)
to get live information about load shedding in South Africa.

It currently only supports Sunsynk single-phase inverters (Deye inverters too,
since they're the same thing underneath).

## Compilation

1. Install Rust e.g. using [these instructions](https://www.rust-lang.org/learn/get-started).
2. Run `cargo install socit` to install the binary. Alternatively,
   check out the repository and run `cargo build --release`. This will compile
   the binary to `target/release/socit`.

If you want to cross-compile:

1. Install and set up [cross](https://github.com/cross-rs/cross) e.g. using
   [these
   instructions](https://github.com/cross-rs/cross/wiki/Getting-Started).
2. Run `cross build --release --target=armv7-unknown-linux-gnueabihf` (replace
   with your target architecture).
3. Find the binary in `target/<arch>/release/target`.

I found that running an arm-unknown-linux-gnueabi build on Raspberry Pi
(model 1B) would produce incorrect results. Using arm-unknown-linux-gnueabihf
(note the last two characters, for hard-float) gives correct results and is
also a lot faster.

## Configuration

Configuration is stored in a [TOML](https://toml.io/) file. There is an
[example](socit.toml.example) in the repository, which you can copy and edit to
provide your configuration. It contains detailed instructions on the available
settings.

## Execution

Run the binary (`socit`) and pass the configuration file as the only
command-line parameter. It uses the
[env_logger](https://docs.rs/env_logger/latest/env_logger/) crate for logging,
so you can enable logging by (for example) setting the environment variable
`RUST_LOG=info`.

## Time synchronisation

You should ensure that the system running socit has its time zone correctly
set and its clock synchronised e.g. with NTP. The inverter does not need
to have time correctly set. Socit will compensate for an incorrect inverter
time.

## Algorithm

Periodically (at the time of writing, once per minute) it projects how the
state of charge might change over the next 24 hours, assuming no grid input.
During load-shedding, it uses pessimistic modelling (`max_discharge_power` and
assumes no solar PV), while the rest of the time it uses optimistic
assumptions (`min_discharge_power` and estimates solar PV assuming no cloud).
It then determines a minimum state of charge that the system should have now
to avoid falling below the `minimum_soc` later (if possible).

There are three SoC levels calculated. When above `target_soc_high`, no grid
power is needed. Between `target_soc_low` and `target_soc_high`, grid power
is used for the load, but the battery is not changed, while below
`target_soc_low`, the battery is charged as well. Finally, the value
`alarm_soc` has no internal effect, but in stored in the database can be used
by external alerting tools: if the actual SoC is below `alarm_soc`, then there
is a risk of falling below `minimum_soc`.

## Trickle charge adjustment

As a separate feature, Socit can dynamically adjust your trickle charge setting
to compensate for electro-magnetic interference (EMI) in the readings from your
CT coil. Mine is particularly bad (errors are typically 200-300W and go up to
500W), causing significant export of power if not corrected for. My
non-essentials (geyser and stove) draw either no power or a large amount of
power (kW+), so any calculated non-essential power draw that's less than this
is assumed to be CT coil error, and is compensated for by adjusting the trickle
charge.

Note that this works for me because the EMI consistently causes the coil to
over-read. If it under-reads, this solution will not work for you because the
trickle charge cannot be set to negative values.

## Changelog

### 0.3.1

- Add influxdb export of coil/trickle charge settings.
- Update dependencies.

### 0.3.0

- Add optional control over the trickle charge setting (see above).
- Change time handling: instead of setting the inverter's clock, just take
  the clock skew into account.
- Update dependencies.

### 0.2.1

- Refactor the robust modbus access into a separate crate (modbus-robust).

### 0.2.0

- Add `alarm_soc`
- Make the startup robust, so that if the modbus connection could not
  immediately be established, socit will keep running and repeatly retry.

### 0.1.1

Fix example configuration documentation to note that the connection to mbusd
will be restarted if necessary.

### 0.1.0

First release.

## License

This program is free software: you can redistribute it and/or modify it
under the terms of the GNU General Public License as published by the Free
Software Foundation, either version 3 of the License, or (at your option)
any later version.

This program is distributed in the hope that it will be useful, but WITHOUT
ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
more details.

You should have received a copy of the GNU General Public License along
with this program. If not, see <https://www.gnu.org/licenses/>.
