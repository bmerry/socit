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

//! Prediction of the angle of the sun relative to solar panels
//!
//! This uses a relatively simple model that ignores all kinds of
//! effects:
//! - precession/nutation/frame bias
//! - refraction
//! - light travel time
//! - relativistic effects (aberration, deflection)
//! - polar motion
//! - variable rotation rate of the earth (specifically, dUT1)
//! - the Moon (it treats the Earth-Moon barycentre as the geocentre)
//!
//! Nevertheless it agrees with high-precision astronomy libraries to
//! better than a degree.
//!
//! The orbital parameters and the equations for applying them are taken
//! from <https://ssd.jpl.nasa.gov/planets/approx_pos.html>, table 2a.

// Lots of variables from external equations don't have snake case
#![allow(non_snake_case)]

use chrono::{DateTime, TimeZone};
use std::f64::consts::PI;
use std::ops::{Index, IndexMut, Mul, Neg};
use std::slice::SliceIndex;

#[derive(PartialEq, Default, Copy, Clone, Debug)]
struct Vector([f64; 3]);

impl<I: SliceIndex<[f64]>> Index<I> for Vector {
    type Output = I::Output;

    fn index(&self, index: I) -> &Self::Output {
        &self.0[index]
    }
}

impl<I: SliceIndex<[f64]>> IndexMut<I> for Vector {
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        &mut self.0[index]
    }
}

impl Mul<f64> for Vector {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Vector(self.0.map(|x| x * rhs))
    }
}

impl Neg for Vector {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Vector(self.0.map(|x| -x))
    }
}

impl Vector {
    fn iter(&self) -> std::slice::Iter<'_, f64> {
        self.0.iter()
    }

    /// Scale to unit length
    fn normalized(&self) -> Self {
        *self * (1.0 / dot(self, self)).sqrt()
    }
}

fn dot(a: &Vector, b: &Vector) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| x * y)
        .fold(0.0, |x, y| x + y)
}

fn cross(a: &Vector, b: &Vector) -> Vector {
    Vector([
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ])
}

#[derive(PartialEq, Default, Copy, Clone, Debug)]
struct Matrix([[f64; 3]; 3]);

impl Index<(usize, usize)> for Matrix {
    type Output = f64;

    fn index(&self, (r, c): (usize, usize)) -> &Self::Output {
        &self.0[r][c]
    }
}

impl IndexMut<(usize, usize)> for Matrix {
    fn index_mut(&mut self, (r, c): (usize, usize)) -> &mut Self::Output {
        &mut self.0[r][c]
    }
}

impl Mul<Vector> for Matrix {
    type Output = Vector;

    fn mul(self, rhs: Vector) -> Self::Output {
        Vector(self.0.map(|x| dot(&Vector(x), &rhs)))
    }
}

/// Solve for E in Kepler's equation M = E - e sin E.
fn kepler(M: f64, e: f64) -> f64 {
    let mut E = M - e * M.sin();
    loop {
        let dM = M - (E - e * E.sin());
        let dE = dM / (1.0 - e * E.cos());
        E += dE;
        if dE.abs() < 1e-8 {
            return E;
        }
    }
}

/// Rotate around the X axis
fn Rx(r: f64) -> Matrix {
    let (s, c) = r.sin_cos();
    Matrix([[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]])
}

/// Rotate around the Z axis
fn Rz(r: f64) -> Matrix {
    let (s, c) = r.sin_cos();
    Matrix([[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]])
}

/// Convert time to a Julian Date relative to some base time (given as UNIX time)
fn timestamp_f64<Tz: TimeZone>(time: &DateTime<Tz>, epoch: f64) -> f64 {
    ((time.timestamp() as f64 - epoch) + 1e-9 * (time.timestamp_subsec_nanos() as f64)) / 86400.0
}

fn earth_rotation_angle<Tz: TimeZone>(time: &DateTime<Tz>) -> f64 {
    // This ignores the difference between UTC and UT1. As such, there
    // isn't too much point worrying about the loss of precision in
    // presenting time as a single floating-point value.
    // This timestamp is relative to 2000-01-1T12:00:00 UTC, ignoring
    // leap seconds.
    let t = timestamp_f64(time, 946728000.0);
    (t.fract() + 0.779057273264 + 0.00273781191135448 * t).fract() * 2.0 * PI
}

/// Normalize angle to [-PI, PI]
fn wrap_angle(x: f64) -> f64 {
    (x + PI).rem_euclid(2.0 * PI) - PI
}

/// Direction from location to the sun, in east-north-up coordinate frame
fn sun_direction<Tz: TimeZone>(lat: f64, lon: f64, time: &DateTime<Tz>) -> Vector {
    const J2000_EPOCH: f64 = 946727935.816;
    const OBLIQUITY: f64 = 23.43928 * PI / 180.0; // to_radians isn't a const function

    // Orbital elements, from NASA model
    let T = timestamp_f64(time, J2000_EPOCH) / 36525.0; // centuries
    let e = 0.01673163 - 0.00003661 * T;
    let I = (-0.00054346 - 0.01337178 * T).to_radians();
    let L = (100.46691572 + 35999.37306329 * T).to_radians();
    let ω_bar = (102.93005885 + 0.31795260 * T).to_radians();
    let Ω = (-5.11260389 - 0.24123856 * T).to_radians();

    let ω = ω_bar - Ω;
    let M = wrap_angle(L - ω_bar);
    let E = kepler(M, e);
    let r_orbital = Vector([E.cos() - e, (1.0 - e * e).sqrt() * E.sin(), 0.0]);
    let r_eq = Rx(-OBLIQUITY) * (Rz(-Ω) * (Rx(-I) * (Rz(-ω) * r_orbital)));
    // Ignore precession-nutation and frame bias. Sign flip is to get Sun
    // relative to Earth instead of vice versa (ignoring the difference between
    // the geocentre and the Earth-Moon barycentre).
    let r_cirs = -r_eq;
    let era = earth_rotation_angle(time);
    let r_tirs = Rz(era) * r_cirs;
    let (slat, clat) = lat.sin_cos();
    let (slon, clon) = lon.sin_cos();
    let l_z = Vector([clat * clon, clat * slon, slat]);
    let l_x = cross(&Vector([0.0, 0.0, 1.0]), &l_z).normalized();
    let l_y = cross(&l_z, &l_x);
    Matrix([l_x.0, l_y.0, l_z.0]) * r_tirs.normalized() // ignores TIRS -> ITRS corrections
}

/// Compute fraction of peak energy for a solar panel with given elevation and azimuth
pub fn solar_fraction<Tz: TimeZone>(
    lat: f64,
    lon: f64,
    elevation: f64,
    azimuth: f64,
    time: &DateTime<Tz>,
) -> f64 {
    let sun_dir = sun_direction(lat, lon, time);
    if sun_dir[2] <= 0.0 {
        return 0.0; // below horizon
    }
    let (s_el, c_el) = elevation.sin_cos();
    let (s_az, c_az) = azimuth.sin_cos();
    let panel_dir = Vector([c_el * s_az, c_el * c_az, s_el]);
    dot(&sun_dir, &panel_dir).max(0.0)
}
