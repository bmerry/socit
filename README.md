# State-of-charge control

This package actively controls the minimum state of charge (battery level) to
adapt to load shedding and to reduce unnecessary cycling of the battery. It
integrates with the [EskomSePush API](https://eskomsepush.gumroad.com/l/api)
to get live information about load shedding in South Africa.

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

Configuration is stored in a [TOML](https://toml.io/) file called
`socit.toml`. There is an [example](socit.toml.example) in the repository,
which you can copy to `socit.toml` and edit to provide your configuration. It
contains detailed instructions on the available settings.

## Algorithm

Periodically (at the time of writing, once per minute) it projects how the
state of charge might change over the next 24 hours, assuming no grid input.
During load-shedding, it uses pessimistic modelling (`max_discharge_power` and
assumes no solar PV), while the rest of the time it uses optimistic
assumptions (`min_discharge_power` and estimates solar PV assuming no cloud).
It then determines a minimum state of charge that the system should have now
to avoid falling below the `minimum_soc` later (if possible).

## Time synchronisation

You should ensure that the system running socit has its time zone correctly
set and its clock synchronised e.g. with NTP. Socit periodically updates the
time on the inverter to match that of the host (so the inverter clock will
stay in sync even without the official dongle).
