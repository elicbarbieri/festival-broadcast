//---------------------------------------------------------------------------------------------------- Use
//use anyhow::{anyhow,bail,ensure};
//use log::{info,error,warn,trace,debug};
//use serde::{Serialize,Deserialize};
//use crate::macros::*;
//use disk::prelude::*;
//use disk::{};
//use std::{};
use std::sync::Arc;
use crate::collection::{
	Artist,Album,Song,QueueKey,
	ArtistKey,AlbumKey,SongKey,Key,
	Collection,Keychain,
};
use rolock::RoLock;
use std::path::PathBuf;
use crate::kernel::{
	ResetState,Kernel,SearchKind,
};
use crate::audio::{Volume, Append, Shuffle, Repeat};
use readable::Percent;

//---------------------------------------------------------------------------------------------------- Kernel Messages.
/// Messages `Frontend` can send to [`Kernel`]
///
/// This is the "API" that all frontends must implement
/// in order to communicate with `Festival`'s internals.
///
/// You can treat these as "commands" sent to [`Kernel`].
pub enum FrontendToKernel {
	// Audio playback.
	/// Toggle playback.
	Toggle,
	/// Play current song.
	Play,
	/// Pause playback.
	Pause,
	/// Play the next song in queue (stop if none).
	Next,
	/// Play the previous song in queue (restart if at first song).
	Previous,

	// Audio settings.
	/// See [`Shuffle`] for the different ways to shuffle.
	Shuffle(Shuffle),
	/// See [`Repeat`] for the different ways to repeat.
	Repeat(Repeat),
	/// Change the audio volume.
	Volume(Volume),

	// Queue.
	/// - [`SongKey`]: add this `Song` to the queue.
	/// - [`Append`]: in which way should we append to the queue?
	/// - [`bool`]: should we clear the queue before appending?
	AddQueueSong((SongKey, Append, bool)),
	/// - [`AlbumKey`]: add all the songs in this `Album` to the queue.
	/// - [`Append`]: in which way should we append to the queue?
	/// - [`bool`]: should we clear the queue before appending?
	/// - [`usize`]: Within this `Album`, should we start at an offset?
	///   e.g, starting at the first `Song` would be 0, starting at the 3rd
	///   `Song` would be offset 2, etc.
	///
	/// If the offset is out of bounds, we will start at the first `Song`.
	AddQueueAlbum((AlbumKey, Append, bool, usize)),
	/// - [`ArtistKey`]: add all the songs by this `Artist` to the queue.
	/// - [`Append`]: in which way should we append to the queue?
	/// - [`bool`]: should we clear the queue before appending?
	/// - [`usize`]: Within this `Artist`, should we start at an offset?
	///   e.g, starting at the first `Song` would be 0, starting at the 3rd
	///   `Song` would be offset 2, etc.
	///
	/// If the offset is out of bounds, we will start at the first `Song`.
	AddQueueArtist((ArtistKey, Append, bool, usize)),
	/// Seek to this second in current song.
	///
	/// This will end the song if the [`usize`] is
	/// greater than the total runtime of the [`Song`].
	Seek(usize),
	/// Skip `usize` amount of `Song`'s.
	///
	/// This doesn't delete the skipped song from the queue, it just skips playback.
	///
	/// If the `usize` is larger than the current `Queue` size, we finish playback.
	Skip(usize),
	/// Same as `Skip` but backwards.
	///
	/// This doesn't delete the skipped song from the queue, it just skips playback.
	///
	/// If the `usize` goes further back than the `Queue` size, we play the first index.
	Back(usize),

	// Queue Index.
	/// - [`usize`]: set the current `Song` to the `n`'th index [`Song`]
	/// in the queue without adding/removing anything.
	///
	/// This will do nothing if the index is out of bounds.
	SetQueueIndex(usize),
	/// Remove a range of queue indices.
	///
	/// This will do nothing if the start or end is out of bounds.
	RemoveQueueRange(std::ops::Range<usize>),

	// Audio State.
	/// We just started up, restore the previous audio
	/// state from disk if there is any.
	RestoreAudioState,

	// Collection.
	/// I'd like a new [`Collection`], scanning these [`PathBuf`]'s for audio files.
	NewCollection(Vec<PathBuf>),
	/// I'd like to search the [`Collection`] with this [`String`] for similar
	/// [`Artist`]'s, [`Album`]'s, and [`Song`]'s.
	///
	/// # Notes
	/// [`Kernel`] will respond with [`KernelToFrontend::SearchResp`].
	Search((String, SearchKind)),

	// Exiting.
	/// I'm exiting, save everything.
	///
	/// # Notes
	/// After you send this message, [`Kernel`] will save everything, and respond with a
	/// [`KernelToFrontend::Exit`] that contains either a [`Result::Ok`] meaning everything went okay,
	/// or [`Result::Err`] with a [`String`] payload containing an error message.
	///
	/// After the response (regardless of the [`Result`]), [`Kernel`] will
	/// - [`std::thread::park`] forever
	/// - Ignore all channel messages
	///
	/// After you receive the response, you should [`std::process::exit`] to kill all threads.
	Exit,
}

/// Messages [`Kernel`] can send to `Frontend`
///
/// This is the "API" that all frontends must implement
/// in order to communicate with `Festival`'s internals.
///
/// You can treat these as "commands" sent _from_ [`Kernel`] that you _**must**_ follow correctly.
///
/// [`Kernel`] assumes that all of these messages are implemented correctly.
///
/// # For example:
/// If your frontend does _not_ actually drop the `Arc<Collection>`
/// after receiving the message [`KernelToFrontend::DropCollection`],
/// then `Festival`'s internals will not be able to destruct the old
/// [`Collection`] correctly.
///
/// This will leave the deconstruction of the old [`Collection`] up to
/// your frontend thread, which is most likely not desired, as it will
/// probably skip a few frames or cause latency.
pub enum KernelToFrontend {
	// Collection.
	/// Drop your [`Arc`] pointer to the [`Collection`].
	DropCollection,
	/// Here's the new [`Collection`] pointer.
	NewCollection(Arc<Collection>),
	/// Creating the new [`Collection`] failed, here's the old pointer and error message.
	Failed((Arc<Collection>, String)),

	// Audio error.
	/// The device error'ed during initialization.
	DeviceError(String),
	/// There was an error while attempting to play a sound.
	PlayError(String),
	/// There was an error while attempting to seek audio.
	SeekError(String),
	/// Attempting to play this [`SongKey`] has errored (probably doesn't exist).
	PathError((SongKey, String)),

	// Search.
	/// Here's a (similarity) search result.
	///
	/// # Notes
	/// This is a response to [`FrontendToKernel::Search`].
	SearchResp(Keychain),

	// Exit.
	/// You sent a [`FrontendToKernel::Exit`], here is the [`Result`]
	/// of saving the data. I'm going to [`std::thread::park`] forever
	/// after this response and ignore channel messages.
	Exit(Result<(), String>),
}

//---------------------------------------------------------------------------------------------------- TESTS
//#[cfg(test)]
//mod tests {
//  #[test]
//  fn __TEST__() {
//  }
//}
