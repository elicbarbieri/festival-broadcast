//---------------------------------------------------------------------------------------------------- Use
//use anyhow::{bail,ensure,Error};
//use log::{info,error,warn,trace,debug};
use serde::{Serialize,Deserialize};
use std::path::PathBuf;
use readable::Runtime;
use super::{
	Artist,
	Album,
	AlbumKey,
	SongKey,
	Collection
};

//----------------------------------------------------------------------------------------------------
#[derive(Debug,Serialize,Deserialize)]
/// Struct holding [`Song`] metadata, with a pointer to the [`Album`] it belongs to
///
/// This struct holds all the metadata about a particular [`Song`].
///
/// It contains a [`SongKey`] that is the index of the owning [`Album`], in the [`Collection`].
pub struct Song {
	// User-facing data.
	/// Title of the [`Song`].
	pub title: String,
	/// Key to the [`Album`].
	pub album: AlbumKey,
	/// Human-readable total runtime of this [`Song`].
	pub runtime_human: Runtime,
	/// The track number of this [`Song`].
	pub track: Option<u32>,
	/// Additional [`Artist`]'s that are on this [`Song`].
	pub track_artists: Option<String>,
	/// The disc number of this [`Song`].
	pub disc: Option<u32>,

	// "Raw" data.
	pub(crate) runtime: f64,
	pub(crate) path: PathBuf,
}

//---------------------------------------------------------------------------------------------------- TESTS
//#[cfg(test)]
//mod tests {
//  #[test]
//  fn _() {
//  }
//}
