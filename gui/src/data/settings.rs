//---------------------------------------------------------------------------------------------------- Use
//use anyhow::{bail,ensure,Error};
//use log::{info,error,warn,trace,debug};
use serde::{Serialize,Deserialize};
use bincode::{Encode,Decode};
use super::{
	Tab,
};
use std::path::PathBuf;
use crate::constants::{
	GUI,
	SETTINGS_VERSION,
	ALBUM_ART_SIZE_MIN,
	ALBUMS_PER_ROW_MIN,
	ALBUM_ART_SIZE_MAX,
	ALBUMS_PER_ROW_MAX,
	ALBUM_ART_SIZE_DEFAULT,
	ALBUMS_PER_ROW_DEFAULT,
	ACCENT_COLOR,
	VISUALS,
};
use shukusai::{
	constants::{
		FESTIVAL,
		STATE_SUB_DIR,
		HEADER,
	},
	collection::{
		Collection,
	},
	sort::{
		ArtistSort,
		AlbumSort,
		SongSort,
	},
	search::SearchKind,
};
use crate::data::{
	AlbumSizing,
	SearchSort,
	ArtistSubTab,
	WindowTitle,
};
use const_format::formatcp;
use std::marker::PhantomData;

//---------------------------------------------------------------------------------------------------- Settings
#[cfg(debug_assertions)]
disk::json!(Settings, disk::Dir::Data, FESTIVAL, formatcp!("{GUI}/{STATE_SUB_DIR}"), "settings");
#[cfg(not(debug_assertions))]
disk::bincode2!(Settings, disk::Dir::Data, FESTIVAL, formatcp!("{GUI}/{STATE_SUB_DIR}"), "settings", HEADER, SETTINGS_VERSION);
#[derive(Clone,Debug,Default,PartialEq,Serialize,Deserialize,Encode,Decode)]
/// `GUI`'s settings.
///
/// Holds user-mutable `GUI` settings, e.g:
/// - Accent color
/// - Album art size
/// - etc
pub struct Settings {
	/// Collection sorting of artist view.
	pub artist_sort: ArtistSort,

	/// Collection sorting of album view.
	pub album_sort: AlbumSort,

	/// Collection sorting of album view.
	pub song_sort: SongSort,

	/// Which search kind to use for `Kernel`
	pub search_kind: SearchKind,

	/// Which `ArtistSubTab` are we on?
	pub artist_sub_tab: ArtistSubTab,

	/// To sort by `Song` title or
	/// `Artist` name in the search tab?
	pub search_sort: SearchSort,

	/// Which way to set the window title when changing songs.
	pub window_title: WindowTitle,

	/// Does the user want a certain amount of
	/// `Album`'s per row or a static pixel size?
	pub album_sizing: AlbumSizing,
	pub album_pixel_size: f32,
	pub albums_per_row: u8,

	/// How many seconds does a song need to play
	/// before the `Previous` button resets the current
	/// instead of going to the previous?
	pub previous_threshold: u32,

	/// Restore playback on re-open.
	pub restore_state: bool,

	/// Start playback if we added stuff to an empty queue.
	pub empty_autoplay: bool,

	#[bincode(with_serde)]
	/// Our accent color.
	pub accent_color: egui::Color32,

	/// List of [`PathBuf`]'s to source music
	/// data from when making a new [`Collection`].
	pub collection_paths: Vec<PathBuf>,

	// Reserved fields.
	_reserved1: PhantomData<Vec<String>>,
	_reserved2: PhantomData<String>,
	_reserved3: PhantomData<Option<String>>,
	_reserved4: PhantomData<bool>,
	_reserved5: PhantomData<bool>,
	_reserved6: PhantomData<Option<bool>>,
	_reserved7: PhantomData<Option<bool>>,
	_reserved8: PhantomData<usize>,
	_reserved9: PhantomData<usize>,
	_reserved10: PhantomData<Option<usize>>,
	_reserved11: PhantomData<Option<usize>>,
}

impl Settings {
//	/// Returns the accent color in [`Settings`] in tuple form.
//	pub const fn accent_color(&self) -> (u8, u8, u8) {
//		let (r, g, b, _) = self.visuals.selection.bg_fill.to_tuple();
//		(r, g, b)
//	}

	pub fn new() -> Self {
		Self {
			accent_color: ACCENT_COLOR,
			restore_state: true,
			collection_paths: vec![],
			album_pixel_size: ALBUM_ART_SIZE_DEFAULT,
			albums_per_row: ALBUMS_PER_ROW_DEFAULT,
			previous_threshold: 5,
			empty_autoplay: true,
			..Default::default()
		}
	}
}

//---------------------------------------------------------------------------------------------------- TESTS
//#[cfg(test)]
//mod test {
//  #[test]
//  fn _() {
//  }
//}
