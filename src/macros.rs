// Read a `RwLock` or `RoLock` and `.expect()`
macro_rules! lock_read {
	($lock:expr) => {
		$lock.read().expect("Failed to read lock: {}", $lock)
	};
}
pub(crate) use lock_read;

// Write a `RwLock` or `RoLock` and `.expect()`
macro_rules! lock_write {
	($lock:expr) => {
		$lock.write().expect("Failed to write lock: {}", $lock)
	};
}
pub(crate) use lock_write;

// Sleep the current thread for `x` milliseconds
macro_rules! sleep {
    ($millis:expr) => {
		std::thread::sleep(std::time::Duration::from_millis($millis))
    };
}
pub(crate) use sleep;

// Flip a bool in place
macro_rules! flip {
	($b:expr) => {
		match $b {
			true|false => $b = !$b,
		}
	};
}
pub(crate) use flip;

// FORWARDS input to log macros, appended with green "... OK"
macro_rules! ok {
	($($tts:tt)*) => {
		log::info!("{} {} {}{}{}", $($tts)*, "...", "\x1b[1;92m", "OK", "\x1b[0m");
	}
}
pub(crate) use ok;

macro_rules! ok_debug {
	($($tts:tt)*) => {
			log::debug!("{} {} {}{}{}", $($tts)*, "...", "\x1b[1;92m", "OK", "\x1b[0m");
	}
}
pub(crate) use ok_debug;

macro_rules! ok_trace {
	($($tts:tt)*) => {
			log::trace!("{} {} {}{}{}", $($tts)*, "...", "\x1b[1;92m", "OK", "\x1b[0m");
	}
}
pub(crate) use ok_trace;

// FORWARDS input to info!() appended with white "... SKIP"
macro_rules! skip {
	($($tts:tt)*) => {
		log::info!("{} {} {}{}{}", $($tts)*, "...", "\x1b[1;97m", "SKIP", "\x1b[0m");
	}
}
pub(crate) use skip;

// FORWARDS input to error!() appended with red "... FAIL"
macro_rules! fail {
	($($tts:tt)*) => {
		log::error!("{} {} {}{}{}", $($tts)*, "...", "\x1b[1;91m", "FAIL", "\x1b[0m");
	}
}
pub(crate) use fail;

// | mass_panic | Logs an error message and terminates all threads           | error!(...); std::process::exit(111)                       |
macro_rules! mass_panic {
	($($tts:tt)*) => {{
		// Log.
		log::error!("");
		log::error!("");
		log::error!("");
		log::error!("----- THREAD PANIC -----");
		log::error!("{}", $($tts)*);
		log::error!("{}", $($tts)*);
		log::error!("{}", $($tts)*);
		log::error!("{}", $($tts)*);
		log::error!("{}", $($tts)*);
		log::error!("{}", $($tts)*);
		log::error!("----- THREAD PANIC -----");
		log::error!("");
		log::error!("");
		log::error!("");

		// Exit all threads.
		std::process::exit(111)
	}}
}
pub(crate) use mass_panic;

// Send a message through a channel, `mass_panic!` on failure
macro_rules! send {
	($channel:expr, $($msg:tt)*) => {{
		if let Err(e) = $channel.send($($msg)*) {
			mass_panic!(e);
		}
	}}
}
pub(crate) use send;

// Receive a message through a channel, `mass_panic!` on failure
macro_rules! recv {
	($channel:expr) => {
		match $channel.recv() {
			Ok(msg) => msg,
			Err(e)  => mass_panic!(e),
		}
	}
}
pub(crate) use recv;

//---------------------------------------------------------------------------------------------------- TESTS
#[cfg(test)]
mod tests {
	#[test]
	fn flip() {
		let mut b = true;
		flip!(b);
		assert!(b == false);
	}
}
