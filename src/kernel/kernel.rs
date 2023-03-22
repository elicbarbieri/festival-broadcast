//---------------------------------------------------------------------------------------------------- Use
use anyhow::{anyhow,bail,ensure,Error};
use log::{info,error,warn,trace,debug};
use serde::{Serialize,Deserialize};
//use crate::macros::*;
//use disk::prelude::*;
//use disk::{};
use std::sync::{Arc,RwLock};
use super::state::State;
use super::volume::Volume;
use rolock::RoLock;
use crate::macros::{
	lock_write,
	lock_read,
	ok_debug,
	recv,
	send,
	flip,
};
use disk::Bincode;
use super::{KernelToFrontend, FrontendToKernel};
use crate::{
	ccd::{KernelToCcd, CcdToKernel, Ccd},
	search::{KernelToSearch, SearchToKernel, Search},
	audio::{KernelToAudio, AudioToKernel, Audio},
	watch::{WatchToKernel, Watch},
	collection::Collection,
};
use crossbeam_channel::{Sender,Receiver};
use std::path::PathBuf;

//---------------------------------------------------------------------------------------------------- Kernel
/// The `Kernel` of `Festival`
///
/// `Kernel`, the messenger and coordinator.
///
/// `Kernel` handles all of `Festival`'s internals and acts
/// as a small & simple interface to all the frontends.
pub struct Kernel {
	// Frontend (GUI) Channels.
	to_frontend: Sender<KernelToFrontend>,
	from_frontend: Receiver<FrontendToKernel>,

	// Search Channels.
	to_search: Sender<KernelToSearch>,
	from_search: Receiver<SearchToKernel>,

	// Audio Channels.
	to_audio: Sender<KernelToAudio>,
	from_audio: Receiver<AudioToKernel>,

	// Watch Channel.
	from_watch: Receiver<WatchToKernel>,

	// Data.
	collection: Arc<Collection>,
	state: Arc<RwLock<super::State>>,
}

// `Kernel` boot process:
//
//`bios()` ---> `boot_loader()` ---> `kernel()` ---> `init()` ---> `userspace()`
//         |                                          |
//         |--- (bios error occured, skip to init) ---|
//
impl Kernel {
	//-------------------------------------------------- bios()
	#[inline(always)]
	// `main()` starts `Kernel` with this.
	pub fn bios(to_frontend: Sender<KernelToFrontend>, from_frontend: Receiver<FrontendToKernel>) {
		debug!("Kernel [1/12] ... entering bios()");

		// Attempt to load `Collection` from file.
		match Collection::from_file() {
			// If success, continue to `boot_loader` to convert
			// bytes to actual usable `egui` images.
			Ok(collection) => Self::boot_loader(collection, to_frontend, from_frontend),

			// Else, straight to `init` with default flag set.
			Err(e) => Self::init(None, None, to_frontend, from_frontend),
		}
	}

	//-------------------------------------------------- boot_loader()
	#[inline(always)]
	fn boot_loader(
		collection: Collection,
		to_frontend: Sender<KernelToFrontend>,
		from_frontend: Receiver<FrontendToKernel>
	) {
		debug!("Kernel [2/12] ... entering boot_loader()");

		// We successfully loaded `Collection`.
		// Create `CCD` channel + thread and make it convert images.
		debug!("Kernel [3/12] ... spawning CCD");
		let (ccd_send, from_ccd) = crossbeam_channel::unbounded::<CcdToKernel>();
		std::thread::spawn(move || Ccd::convert_art(ccd_send, collection));

		// Before hanging on `CCD`, read `State` file.
		// Note: This is a `Result`.
		debug!("Kernel [4/12] ... reading State");
		let state = State::from_file();

		// Wait for `Collection` to be returned by `CCD`.
		debug!("Kernel [5/12] ... waiting on CCD");
		let collection = loop {
			use CcdToKernel::*;
			match recv!(from_ccd) {
				NewCollection(collection) => break collection,
				Failed(string)            => (), // TODO: Forward to `GUI`.
				Update(string)            => (), // TODO: Forward to `GUI`.
			}
		};

		// Continue to `kernel` to verify data.
		Self::kernel(collection, state, to_frontend, from_frontend);
	}

	//-------------------------------------------------- kernel()
	#[inline(always)]
	fn kernel(
		collection:    Collection,
		state:         Result<State, anyhow::Error>,
		to_frontend:   Sender<KernelToFrontend>,
		from_frontend: Receiver<FrontendToKernel>,
	) {
		/* TODO: initialize and sanitize collection & misc data */
		debug!("Kernel [6/12] ... entering kernel()");
		let state = state.unwrap();

		Self::init(Some(collection), Some(state), to_frontend, from_frontend);
	}

	//-------------------------------------------------- init()
	#[inline(always)]
	fn init(
		collection:    Option<Collection>,
		state:         Option<State>,
		to_frontend:   Sender<KernelToFrontend>,
		from_frontend: Receiver<FrontendToKernel>
	) {
		debug!("Kernel [7/12] ... entering init()");

		// Handle potentially missing `Collection`.
		let collection = match collection {
			Some(c) => { debug!("Kernel [8/12] ... Collection found"); Arc::new(c) },
			None    => { debug!("Kernel [8/12] ... Collection NOT found, returning default"); Arc::new(Collection::new()) },
		};

		// Handle potentially missing `State`.
		let state = match state {
			Some(s) => { debug!("Kernel [9/12] ... State found"); Arc::new(RwLock::new(s)) },
			None    => { debug!("Kernel [9/12] ... State NOT found, returning default"); Arc::new(RwLock::new(State::new())) },
		};

		// Create `To` channels.
		let (to_search, search_recv) = crossbeam_channel::unbounded::<KernelToSearch>();
		let (to_audio,  audio_recv)  = crossbeam_channel::unbounded::<KernelToAudio>();

		// Create `From` channels.
		let (search_send, from_search) = crossbeam_channel::unbounded::<SearchToKernel>();
		let (audio_send,  from_audio)  = crossbeam_channel::unbounded::<AudioToKernel>();
		let (watch_send,  from_watch)  = crossbeam_channel::unbounded::<WatchToKernel>();

		// Create `Kernel`.
		let kernel = Self {
			// Channels.
			to_frontend, from_frontend,
			to_search, from_search,
			to_audio, from_audio,
			from_watch,

			// Data.
			collection, state,
		};

		// Spawn `Search`.
		debug!("Kernel [10/12] ... spawning Search");
		let collection = Arc::clone(&kernel.collection);
		std::thread::spawn(move || Search::init(collection, search_send, search_recv));

		// Spawn `Audio`.
		debug!("Kernel [11/12] ... spawning Audio");
		let collection = Arc::clone(&kernel.collection);
		let state      = RoLock::new(&kernel.state);
		std::thread::spawn(move || Audio::init(collection, state, audio_send, audio_recv));

		// Spawn `Watch`.
		debug!("Kernel [12/12] ... spawning Watch");
		std::thread::spawn(move || Watch::init(watch_send));

		// We're done, enter main `userspace` loop.
		debug!("Kernel: entering userspace()");
		Self::userspace(kernel);
	}

}

//---------------------------------------------------------------------------------------------------- Main Kernel loop (userspace)
impl Kernel {
	#[inline(always)]
	fn userspace(mut self) {
		ok_debug!("Kernel");

		// Array of our channels we can `select` from.
		let mut select = crossbeam_channel::Select::new();
		// FIXME:
		// These channels need to be cloned first because
		// `select.recv()` requires a `&`, but we need a
		// `&mut` version of `self` later, so instead,
		// we give `select.recv()` a cloned `&`.
		let (frontend, search, audio, watch) = (
			self.from_frontend.clone(),
			self.from_search.clone(),
			self.from_audio.clone(),
			self.from_watch.clone(),
		);
		let (frontend, search, audio, watch) = (
			select.recv(&frontend),
			select.recv(&search),
			select.recv(&audio),
			select.recv(&watch),
		);

		// 1) Hang until message is ready.
		// 2) Receive the message and pass to appropriate function.
		// 3) Loop.
		loop {
			match select.ready() {
				i if i == frontend => self.msg_frontend(recv!(self.from_frontend)),
				i if i == search   => self.msg_search(recv!(self.from_search)),
				i if i == audio    => self.msg_audio(recv!(self.from_audio)),
				i if i == watch    => self.msg_watch(recv!(self.from_watch)),
				_ => error!("Kernel: Received an unknown message"),
			}
		}
	}

	//-------------------------------------------------- Message handling.
	#[inline(always)]
	// We got a message from `GUI`.
	fn msg_frontend(&mut self, msg: FrontendToKernel) {
		use super::FrontendToKernel::*;
		match msg {
			// Audio playback.
			Play                 => send!(self.to_audio, KernelToAudio::Play),
			Stop                 => send!(self.to_audio, KernelToAudio::Stop),
			Next                 => send!(self.to_audio, KernelToAudio::Next),
			Last                 => send!(self.to_audio, KernelToAudio::Last),
			Seek(float)          => self.seek(float),
			PlayQueueKey(key)    => send!(self.to_audio, KernelToAudio::PlayQueueKey(key)),
			Volume(volume)       => send!(self.to_audio, KernelToAudio::Volume(volume.inner())),
			// Audio settings.
			Shuffle              => flip!(lock_write!(self.state).shuffle),
			Repeat               => flip!(lock_write!(self.state).repeat),
			// Collection.
			NewCollection(paths) => self.ccd_mode(paths),
			Search(string)       => send!(self.to_search, KernelToSearch::Search(string)),
		}
	}

	#[inline(always)]
	// We got a message from `Search`.
	fn msg_search(&self, msg: SearchToKernel) {
		use crate::search::SearchToKernel::*;
		match msg {
			SearchResult(keychain) => send!(self.to_frontend, KernelToFrontend::SearchResult(keychain)),
		}
	}

	#[inline(always)]
	// We got a message from `Audio`.
	fn msg_audio(&self, msg: AudioToKernel) {
		use crate::audio::AudioToKernel::*;
		match msg {
			TimestampUpdate(float) => lock_write!(self.state).current_runtime = float,
			PathError(string)      => send!(self.to_frontend, KernelToFrontend::PathError(string)),
		}
	}

	#[inline(always)]
	// We got a message from `Watch`.
	fn msg_watch(&self, msg: WatchToKernel) {
		use crate::watch::WatchToKernel::*;
		match msg {
			Play    => send!(self.to_audio, KernelToAudio::Play),
			Stop    => send!(self.to_audio, KernelToAudio::Stop),
			Next    => send!(self.to_audio, KernelToAudio::Next),
			Last    => send!(self.to_audio, KernelToAudio::Last),
			Shuffle => flip!(lock_write!(self.state).shuffle),
			Repeat  => flip!(lock_write!(self.state).repeat),
		}
	}

	//-------------------------------------------------- Misc message handling.
	#[inline(always)]
	// Verify the `seek` is valid before sending to `Audio`.
	fn seek(&self, float: f64) {
		if !lock_read!(self.state).playing {
			return
		}

		if float <= lock_read!(self.state).current_runtime {
			send!(self.to_audio, KernelToAudio::Play);
		}
	}

	//-------------------------------------------------- `CCD` Mode.
	#[inline(always)]
	// `GUI` wants a new `Collection`:
	//
	// 1. Enter `CCD` mode
	// 2. Only listen to it
	// 3. (but send updates to `GUI`)
	// 4. Tell everyone to drop the old `Collection` pointer
	// 5. Wait until `CCD` gives the new `Collection`
	// 6. Tell `CCD` to... `Die`
	// 7. Give new `Arc<Collection>` to everyone
	fn ccd_mode(&mut self, paths: Vec<PathBuf>) {
		// INVARIANT:
		// `GUI` is expected to drop its pointer by itself
		// after requesting the new `Collection`.
		//
		// Drop your pointers.
		send!(self.to_search, KernelToSearch::DropCollection);
		send!(self.to_audio,  KernelToAudio::DropCollection);

		// Create `CCD` channels.
		let (to_ccd,   ccd_recv) = crossbeam_channel::unbounded::<KernelToCcd>();
		let (ccd_send, from_ccd) = crossbeam_channel::unbounded::<CcdToKernel>();

		// Get old `Collection` pointer.
		let old_collection = Arc::clone(&self.collection);

		// Spawn `CCD`.
		std::thread::spawn(move || {
			Ccd::new_collection(ccd_send, ccd_recv, old_collection, paths);
		});

		// Listen to `CCD`.
		let collection = loop {
			use crate::ccd::CcdToKernel::*;
			match recv!(from_ccd) {
				Update(string)            => send!(self.to_frontend, KernelToFrontend::Update(string)),
				NewCollection(collection) => break collection,
				Failed(anyhow)            => {
					// `CCD` failed, tell `GUI` and give the
					// old `Collection` pointer to everyone again.
					send!(self.to_search, KernelToSearch::NewCollection(Arc::clone(&self.collection)));
					send!(self.to_audio,  KernelToAudio::NewCollection(Arc::clone(&self.collection)));
					send!(self.to_frontend,    KernelToFrontend::Failed((Arc::clone(&self.collection), anyhow.to_string())));
					return;
				},
			}
		};

		// `CCD` succeeded, send new pointers to everyone.
		self.collection = Arc::new(collection);
		send!(self.to_search, KernelToSearch::NewCollection(Arc::clone(&self.collection)));
		send!(self.to_audio,  KernelToAudio::NewCollection(Arc::clone(&self.collection)));
		send!(self.to_frontend,    KernelToFrontend::NewCollection(Arc::clone(&self.collection)));
	}
}

//---------------------------------------------------------------------------------------------------- TESTS
//#[cfg(test)]
//mod tests {
//  #[test]
//  fn __TEST__() {
//  }
//}
