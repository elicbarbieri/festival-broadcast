use serde::{Serialize, Deserialize};

//---------------------------------------------------------------------------------------------------- Constants
// The locale numbers are formatting in is English, which looks like: [1,000]
pub const LOCALE: num_format::Locale = num_format::Locale::en;
pub const ZERO_SECONDS: std::time::Duration = std::time::Duration::from_secs(0);

//---------------------------------------------------------------------------------------------------- [HumanTime]
/// This converts a [`std::time::Duration`] into something more readable.
/// Used for uptime display purposes: `7 years, 8 months, 15 days, 23 hours, 35 minutes, 1 second`
/// Code taken from [`https://docs.rs/humantime`] and edited to remove sub-second time, change spacing and some words.
use std::time::Duration;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct HumanTime(Duration);

impl Default for HumanTime {
	fn default() -> Self {
		Self::new()
	}
}

impl HumanTime {
	pub const fn new() -> HumanTime {
		HumanTime(ZERO_SECONDS)
	}

	pub const fn into_human(d: Duration) -> HumanTime {
		HumanTime(d)
	}

	pub const fn from_u64(u: u64) -> HumanTime {
		HumanTime(Duration::from_secs(u))
	}

	fn plural(f: &mut std::fmt::Formatter, started: &mut bool, name: &str, value: u64) -> std::fmt::Result {
		if value > 0 {
			if *started {
				f.write_str(", ")?;
			}
			write!(f, "{} {}", value, name)?;
			if value > 1 {
				f.write_str("s")?;
			}
			*started = true;
		}
		Ok(())
	}
}

impl std::fmt::Display for HumanTime {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		let secs = self.0.as_secs();
		if secs == 0 {
			f.write_str("0 seconds")?;
			return Ok(());
		}

		let years = secs / 31_557_600;  // 365.25d
		let ydays = secs % 31_557_600;
		let months = ydays / 2_630_016;  // 30.44d
		let mdays = ydays % 2_630_016;
		let days = mdays / 86400;
		let day_secs = mdays % 86400;
		let hours = day_secs / 3600;
		let minutes = day_secs % 3600 / 60;
		let seconds = day_secs % 60;

		let started = &mut false;
		Self::plural(f, started, "year", years)?;
		Self::plural(f, started, "month", months)?;
		Self::plural(f, started, "day", days)?;
		Self::plural(f, started, "hour", hours)?;
		Self::plural(f, started, "minute", minutes)?;
		Self::plural(f, started, "second", seconds)?;
		Ok(())
	}
}

//---------------------------------------------------------------------------------------------------- [HumanNumber]
// Human readable numbers.
// Float    | `1234.57` -> `1,234`                    | Casts as u64/u128, adds comma
// Unsigned | `1234567` -> `1,234,567`                | Adds comma
// Percent  | `99.123`  -> `99.12%`                    | Truncates to 2 after dot, adds percent
// Percent  | `0.001`   -> `0%`                        | Rounds down, removes redundant zeros
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct HumanNumber(String);

impl std::fmt::Display for HumanNumber {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "{}", &self.0)
	}
}

impl HumanNumber {
	pub fn unknown() -> Self {
		Self("???".to_string())
	}
	pub fn from_str(s: &str) -> Self {
		Self(s.to_string())
	}
	pub fn to_percent(f: f32) -> Self {
		if f < 0.01 {
			Self("0%".to_string())
		} else {
			Self(format!("{:.2}%", f))
		}
	}
	pub fn to_percent_3_point(f: f32) -> Self {
		Self(format!("{:.3}%", f))
	}
	pub fn to_percent_no_fmt(f: f32) -> Self {
		Self(format!("{}%", f))
	}
	pub fn from_f64_to_percent_3_point(f: f64) -> Self {
		Self(format!("{:.3}%", f))
	}
	pub fn from_f64_to_percent_6_point(f: f64) -> Self {
		Self(format!("{:.6}%", f))
	}
	pub fn from_f64_to_percent_9_point(f: f64) -> Self {
		Self(format!("{:.9}%", f))
	}
	pub fn from_f64_to_percent_no_fmt(f: f64) -> Self {
		Self(format!("{}%", f))
	}
	pub fn from_f32(f: f32) -> Self {
		let mut buf = num_format::Buffer::new();
		buf.write_formatted(&(f as u64), &LOCALE);
		Self(buf.as_str().to_string())
	}
	pub fn from_f64(f: f64) -> Self {
		let mut buf = num_format::Buffer::new();
		buf.write_formatted(&(f as u128), &LOCALE);
		Self(buf.as_str().to_string())
	}
	pub fn from_u16(u: u16) -> Self {
		let mut buf = num_format::Buffer::new();
		buf.write_formatted(&u, &LOCALE);
		Self(buf.as_str().to_string())
	}
	pub fn from_u32(u: u32) -> Self {
		let mut buf = num_format::Buffer::new();
		buf.write_formatted(&u, &LOCALE);
		Self(buf.as_str().to_string())
	}
	pub fn from_u64(u: u64) -> Self {
		let mut buf = num_format::Buffer::new();
		buf.write_formatted(&u, &LOCALE);
		Self(buf.as_str().to_string())
	}
	pub fn from_u128(u: u128) -> Self {
		let mut buf = num_format::Buffer::new();
		buf.write_formatted(&u, &LOCALE);
		Self(buf.as_str().to_string())
	}
	pub fn from_f64_12_point(f: f64) -> Self {
		let f = format!("{:.12}", f);
		Self(f)
	}
	pub fn from_f64_no_fmt(f: f64) -> Self {
		let f = format!("{}", f);
		Self(f)
	}
	pub fn as_str(&self) -> &str {
		self.0.as_str()
	}
}

//---------------------------------------------------------------------------------------------------- [HumanRuntime]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Human readable "audio/video runtime" in `H:M:S` format, e.g:
/// `11.1111` -> `0:11`
/// `111.111` -> `1:51`
/// `11111.1` -> `3:05:11`
///
/// The struct itself is just a tuple struct holding `(f64, String)`.
///
/// The `f64` represents seconds.
/// The `String` is the human-readable version.
///
/// Formatting rules:
/// 1. `seconds` always has leading `0`.
/// 2. `minutes` only has a leading zero if `hours` isn't `0`.
/// 3. `hours` never has a leading `0`.
///
/// e.g:
/// `1:01:01`
/// `1:01`
/// `0:01`
pub struct HumanRuntime(f64, String);

impl HumanRuntime {
	#[inline]
	/// Returns zero-second `(0, "0:00")` length [`Self`].
	fn zero() -> Self {
		Self(0.0, "0:00".to_string())
	}

	#[inline]
	/// Returns one-second `(1.0, "0:01")` length [`Self`].
	fn one() -> Self {
		Self(1.0, "0:01".to_string())
	}

	#[inline]
	fn as_str(&self) -> &str {
		self.1.as_str()
	}

	#[inline]
	/// Returns a [`Clone`] of the inner [`String`].
	fn to_string(&self) -> String {
		self.1.to_string()
	}

	#[inline]
	/// Returns a [`Copy`] of the inner [`f64`].
	fn to_f64(&self) -> f64 {
		self.0
	}

	#[inline]
	/// Consumes [`Self`] for the inner [`String`].
	fn into_string(self) -> String {
		self.1
	}

	#[inline]
	/// Consumes [`Self`] for the inner `(f64, String)`.
	fn into_raw(self) -> (f64, String) {
		(self.0, self.1)
	}
}

impl From<f64> for HumanRuntime {
	fn from(runtime: f64) -> Self {
		// Zero length.
		if runtime == 0.0 {
			return Self::zero()
		}

		// Round up to one second length.
		if runtime < 1.0 {
			return Self::one()
		}

		// Cast to `u64` (implicitly rounds down).
	    let seconds = (runtime % 60.0) as u64;
	    let minutes = ((runtime / 60.0) % 60.0) as u64;
	    let hours   = ((runtime / 60.0) / 60.0) as u64;

		// Format.
		let string = if hours > 0 {
			format!("{}:{:0>2}:{:0>2}", hours, minutes, seconds)
		} else {
			format!("{}:{:0>2}", minutes, seconds)
		};

		Self(runtime, string)
	}
}

impl From<f32> for HumanRuntime {
	fn from(runtime: f32) -> Self {
		// Zero length.
		if runtime == 0.0 {
			return Self::zero()
		}

		// Round up to one second length.
		if runtime < 1.0 {
			return Self::one()
		}

		// `f32` -> `f64`.
		let runtime = f64::from(runtime);

		// Cast to `u64` (implicitly rounds down).
	    let seconds = (runtime % 60.0) as u64;
	    let minutes = ((runtime / 60.0) % 60.0) as u64;
	    let hours   = ((runtime / 60.0) / 60.0) as u64;

		// Format.
		let string = if hours > 0 {
			format!("{}:{:0>2}:{:0>2}", hours, minutes, seconds)
		} else {
			format!("{}:{:0>2}", minutes, seconds)
		};

		Self(runtime, string)
	}
}

impl std::fmt::Display for HumanRuntime {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "{}", self.1.as_str())
	}
}

//---------------------------------------------------------------------------------------------------- TESTS
#[cfg(test)]
mod test {
	use super::*;
	use std::time::Duration;

	#[test]
	fn human_number() {
		assert!(HumanNumber::to_percent(0.001).to_string() == "0%");
		assert!(HumanNumber::to_percent(12.123123123123).to_string() == "12.12%");
		assert!(HumanNumber::to_percent_3_point(0.001).to_string() == "0.001%");
		assert!(HumanNumber::from_f32(123_123.123123123).to_string() == "123,123");
		assert!(HumanNumber::from_f64(123_123_123.123123123123123).to_string() == "123,123,123");
		assert!(HumanNumber::from_u16(1_000).to_string() == "1,000");
		assert!(HumanNumber::from_u16(65_535).to_string() == "65,535");
		assert!(HumanNumber::from_u32(65_536).to_string() == "65,536");
		assert!(HumanNumber::from_u32(100_000).to_string() == "100,000");
		assert!(HumanNumber::from_u32(1_000_000).to_string() == "1,000,000");
		assert!(HumanNumber::from_u32(10_000_000).to_string() == "10,000,000");
		assert!(HumanNumber::from_u32(100_000_000).to_string() == "100,000,000");
		assert!(HumanNumber::from_u32(1_000_000_000).to_string() == "1,000,000,000");
		assert!(HumanNumber::from_u32(4_294_967_295).to_string() == "4,294,967,295");
		assert!(HumanNumber::from_u64(4_294_967_296).to_string() == "4,294,967,296");
		assert!(HumanNumber::from_u64(10_000_000_000).to_string() == "10,000,000,000");
		assert!(HumanNumber::from_u64(100_000_000_000).to_string() == "100,000,000,000");
		assert!(HumanNumber::from_u64(1_000_000_000_000).to_string() == "1,000,000,000,000");
		assert!(HumanNumber::from_u64(10_000_000_000_000).to_string() == "10,000,000,000,000");
		assert!(HumanNumber::from_u64(100_000_000_000_000).to_string() == "100,000,000,000,000");
		assert!(HumanNumber::from_u64(1_000_000_000_000_000).to_string() == "1,000,000,000,000,000");
		assert!(HumanNumber::from_u64(10_000_000_000_000_000).to_string() == "10,000,000,000,000,000");
		assert!(HumanNumber::from_u64(18_446_744_073_709_551_615).to_string() == "18,446,744,073,709,551,615");
		assert!(HumanNumber::from_u128(18_446_744_073_709_551_616).to_string() == "18,446,744,073,709,551,616");
		assert!(HumanNumber::from_u128(100_000_000_000_000_000_000).to_string() == "100,000,000,000,000,000,000");
		assert_eq!(
			HumanNumber::from_u128(340_282_366_920_938_463_463_374_607_431_768_211_455).to_string(),
			"340,282,366,920,938,463,463,374,607,431,768,211,455",
		);
	}

	#[test]
	fn human_time() {
		assert!(HumanTime::into_human(Duration::from_secs(0)).to_string()        == "0 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(1)).to_string()        == "1 second");
		assert!(HumanTime::into_human(Duration::from_secs(2)).to_string()        == "2 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(59)).to_string()       == "59 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(60)).to_string()       == "1 minute");
		assert!(HumanTime::into_human(Duration::from_secs(61)).to_string()       == "1 minute, 1 second");
		assert!(HumanTime::into_human(Duration::from_secs(62)).to_string()       == "1 minute, 2 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(120)).to_string()      == "2 minutes");
		assert!(HumanTime::into_human(Duration::from_secs(121)).to_string()      == "2 minutes, 1 second");
		assert!(HumanTime::into_human(Duration::from_secs(122)).to_string()      == "2 minutes, 2 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(179)).to_string()      == "2 minutes, 59 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(3599)).to_string()     == "59 minutes, 59 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(3600)).to_string()     == "1 hour");
		assert!(HumanTime::into_human(Duration::from_secs(3601)).to_string()     == "1 hour, 1 second");
		assert!(HumanTime::into_human(Duration::from_secs(3602)).to_string()     == "1 hour, 2 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(3660)).to_string()     == "1 hour, 1 minute");
		assert!(HumanTime::into_human(Duration::from_secs(3720)).to_string()     == "1 hour, 2 minutes");
		assert!(HumanTime::into_human(Duration::from_secs(86399)).to_string()    == "23 hours, 59 minutes, 59 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(86400)).to_string()    == "1 day");
		assert!(HumanTime::into_human(Duration::from_secs(86401)).to_string()    == "1 day, 1 second");
		assert!(HumanTime::into_human(Duration::from_secs(86402)).to_string()    == "1 day, 2 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(86460)).to_string()    == "1 day, 1 minute");
		assert!(HumanTime::into_human(Duration::from_secs(86520)).to_string()    == "1 day, 2 minutes");
		assert!(HumanTime::into_human(Duration::from_secs(90000)).to_string()    == "1 day, 1 hour");
		assert!(HumanTime::into_human(Duration::from_secs(93600)).to_string()    == "1 day, 2 hours");
		assert!(HumanTime::into_human(Duration::from_secs(604799)).to_string()   == "6 days, 23 hours, 59 minutes, 59 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(604800)).to_string()   == "7 days");
		assert!(HumanTime::into_human(Duration::from_secs(2630016)).to_string()  == "1 month");
		assert!(HumanTime::into_human(Duration::from_secs(3234815)).to_string()  == "1 month, 6 days, 23 hours, 59 minutes, 59 seconds");
		assert!(HumanTime::into_human(Duration::from_secs(5260032)).to_string()  == "2 months");
		assert!(HumanTime::into_human(Duration::from_secs(31557600)).to_string() == "1 year");
		assert!(HumanTime::into_human(Duration::from_secs(63115200)).to_string() == "2 years");
		assert_eq!(
			HumanTime::into_human(Duration::from_secs(18446744073709551615)).to_string(),
			"584542046090 years, 7 months, 15 days, 17 hours, 5 minutes, 3 seconds",
		);
	}

	#[test]
	fn human_runtime() {
		// Always round down.
		assert!(HumanRuntime::from(11.1111).as_str() == "0:11");
		assert!(HumanRuntime::from(11.9999).as_str() == "0:11");

		assert!(HumanRuntime::from(111.111).as_str() == "1:51");
		assert!(HumanRuntime::from(111.999).as_str() == "1:51");

		assert!(HumanRuntime::from(11111.1).as_str() == "3:05:11");
		assert!(HumanRuntime::from(11111.9).as_str() == "3:05:11");

		assert!(HumanRuntime::from(0.0).as_str() == "0:00");
		assert!(HumanRuntime::from(1.0).as_str() == "0:01");
		assert!(HumanRuntime::from(1.9).as_str() == "0:01");
		assert!(HumanRuntime::from(2.0).as_str() == "0:02");
	}
}
