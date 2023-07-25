//---------------------------------------------------------------------------------------------------- Use
use serde::Serialize;
use bincode::{Encode,Decode};
use std::marker::PhantomData;
use readable::Runtime;
use crate::collection::key::{
	AlbumKey,
	SongKey,
};
use std::sync::Arc;

//----------------------------------------------------------------------------------------------------
#[derive(Clone,Debug,Hash,PartialEq,Eq,PartialOrd,Ord,Serialize,Encode,Decode)]
/// Struct holding [`Artist`] metadata, with pointers to [`Album`]\(s\)
///
/// This struct holds all the metadata about a particular [`Artist`].
///
/// It contains an [`Vec`] of [`AlbumKey`]\(s\) that are the indices of the associated [`Album`]\(s\), in the [`Collection`].
pub struct Artist {
	/// The [`Artist`]'s name.
	pub name: Arc<str>,
	/// The [`Artist`]'s name in "Unicode Derived Core Property" lowercase.
	pub name_lowercase: Arc<str>,
	#[serde(serialize_with = "crate::collection::serde::runtime")]
	/// Total runtime.
	pub runtime: Runtime,
	// SOMEDAY:
	// This should be a Box<[AlbumKey]>.
	/// Keys to the associated [`Album`]\(s\).
	pub albums: Vec<AlbumKey>,
	/// Keys to every [`Song`] by this [`Artist`].
	///
	/// The order is [`Album`] release order, then [`Song`] track order.
	pub songs: Box<[SongKey]>,
}

impl Default for Artist {
	fn default() -> Self {
		Self {
			name: "".into(),
			name_lowercase: "".into(),
			runtime: Default::default(),
			albums: Vec::with_capacity(0),
			songs: Box::new([]),
		}
	}
}

//---------------------------------------------------------------------------------------------------- TESTS
#[cfg(test)]
mod tests {
	use super::*;

	const EXPECTED: &str =
r#"{
  "name": "",
  "name_lowercase": "",
  "runtime": 0,
  "albums": [],
  "songs": []
}"#;

	#[test]
	fn serde_json() {
		let d: String = serde_json::to_string_pretty(&Artist::default()).unwrap();
		assert_eq!(EXPECTED, d);
	}
}
