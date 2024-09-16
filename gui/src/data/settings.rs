//---------------------------------------------------------------------------------------------------- Use
//use anyhow::{bail,ensure,Error};
//use log::{info,error,warn,trace,debug};
use std::default::Default;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::__private::kind::TraitKind;
use const_format::formatcp;
use disk::Toml;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use toml::Table;

use eframe::wgpu::core::resource::BufferMapAsyncStatus::AlreadyMapped;
use shukusai::{
  audio::PREVIOUS_THRESHOLD_DEFAULT,
  constants::{
    FESTIVAL,
    HEADER,
    STATE_SUB_DIR,
  },
  search::SearchKind,
  sort::{
    AlbumSort,
    ArtistSort,
    SongSort,
  },
};

use crate::constants::{
  ACCENT_COLOR,
  ALBUM_ART_SIZE_DEFAULT,
  ALBUMS_PER_ROW_DEFAULT,
  AUTO_SAVE_INTERVAL_SECONDS,
  GUI,
  PIXELS_PER_POINT_DEFAULT,
  SETTINGS_VERSION,
};
use crate::data::{AlbumSizing, SearchSort, Tab, WindowTitle};

//---------------------------------------------------------------------------------------------------- Settings
disk::toml!(Settings, disk::Dir::Data, FESTIVAL, formatcp!("{GUI}/{STATE_SUB_DIR}"), "settings");
#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
/// GUI's settings.
///
/// Holds user-mutable GUI settings, e.g:
/// - Accent color
/// - Album art size
/// - etc
pub struct Settings {
  /// Version of the settings file.
  pub settings_version: u8,

  /// Collection sorting of artist view.
  pub artist_sort: ArtistSort,

  /// Collection sorting of album view.
  pub album_sort: AlbumSort,

  /// Collection sorting of album view.
  pub song_sort: SongSort,

  /// Which search kind to use for Kernel
  pub search_kind: SearchKind,

  /// To sort by Song title or
  /// Artist name in the search tab?
  pub search_sort: SearchSort,

  /// Which way to set the window title when changing songs.
  pub window_title: WindowTitle,

  /// Does the user want a certain amount of
  /// Album's per row or a static pixel size?
  pub album_sizing: AlbumSizing,
  pub album_pixel_size: f32,
  pub albums_per_row: u8,

  /// How many seconds does a song need to play
  /// before the Previous button resets the current
  /// instead of going to the previous?
  pub previous_threshold: u32,

  /// Auto-save the audio state to disk every auto_save seconds.
  pub auto_save: u8,

  /// Restore playback on re-open.
  pub restore_state: bool,

  /// Start playback if we added stuff to an empty queue.
  pub empty_autoplay: bool,

  /// Our accent color.
  pub accent_color: egui::Color32,

  /// List of [PathBuf]'s to source music
  /// data from when making a new [Collection].
  pub collection_paths: Vec<PathBuf>,

  /// What egui::Context::pixels_per_point are we set to?
  /// Default is 1.0, this allows the user to scale manually.
  pub pixels_per_point: f32,
}

impl Settings {
  pub fn new() -> Self {
    Self {
      settings_version:   SETTINGS_VERSION,
      artist_sort:        Default::default(),
      album_sort:         Default::default(),
      song_sort:          Default::default(),
      search_kind:        Default::default(),
      search_sort:        Default::default(),
      window_title:       Default::default(),
      album_sizing:       Default::default(),
      album_pixel_size:   ALBUM_ART_SIZE_DEFAULT,
      albums_per_row:     ALBUMS_PER_ROW_DEFAULT,
      previous_threshold: PREVIOUS_THRESHOLD_DEFAULT,
      auto_save:          AUTO_SAVE_INTERVAL_SECONDS,
      restore_state:      true,
      empty_autoplay:     true,
      accent_color:       ACCENT_COLOR,
      collection_paths:   vec![],
      pixels_per_point:   PIXELS_PER_POINT_DEFAULT,
    }
  }

pub fn read_from_disk(path: Option<PathBuf>) -> Result<Self, disk::Error> {
    let path = path.unwrap_or_else(|| Settings::absolute_path().unwrap());

    if !path.exists() { return Ok(Self::new()); }

    match Self::from_path(&path) {
      Ok(settings) => Ok(settings),
      Err(_) => {
        match std::fs::read_to_string(&path) {
          Ok(toml_str) => {
            info!("Reading old settings file");
            let existing_settings = toml::from_str::<Table>(&toml_str)?;
            Ok(Self::load_old_settings(existing_settings))
          },
          Err(e) => {
            error!("Error reading settings file: {}", e);
            Ok(Self::new())
          }
        }
      }
    }
  }

  fn load_old_settings(old_settings: Table) -> Self {
    let old_version = old_settings.get("settings_version").unwrap().as_integer().unwrap();
    info!("Parsing in Existing Version {}", old_version);

    let mut settings = Self::new();
    for (key, value) in old_settings {
      match key.as_str() {
        // TODO: Cleanup the struct settings parse with a macro
        "artist_sort" => settings.artist_sort =  match value.as_str() {Some(v)=>ArtistSort::from_str(v).unwrap_or_default(), None => ArtistSort::default()},
        "album_sort" => settings.album_sort = match value.as_str() {Some(v)=>AlbumSort::from_str(v).unwrap_or_default(), None => AlbumSort::default()},
        "song_sort" => settings.song_sort = match value.as_str() {Some(v)=>SongSort::from_str(v).unwrap_or_default(), None => SongSort::default()},
        "search_kind" => settings.search_kind = match value.as_str() {Some(v)=>SearchKind::from_str(v).unwrap_or_default(), None => SearchKind::default()},
        "search_sort" => settings.search_sort = match value.as_str() {Some(v)=>SearchSort::from_str(v).unwrap_or_default(), None => SearchSort::default()},
        "window_title" => settings.window_title = match value.as_str() {Some(v)=> WindowTitle::from_str(v).unwrap_or_default(), None => WindowTitle::default()},
        "album_sizing" => settings.album_sizing = match value.as_str() {Some(v)=> AlbumSizing::from_str(v).unwrap_or_default(), None => AlbumSizing::default()},
        "album_pixel_size" => settings.album_pixel_size = match value.as_integer() {Some(v)=> v as f32, None => ALBUM_ART_SIZE_DEFAULT},
        "albums_per_row" => settings.albums_per_row = match value.as_integer() {Some(v)=> v as u8, None => ALBUMS_PER_ROW_DEFAULT},
        "previous_threshold" => settings.previous_threshold = match value.as_integer() {Some(v)=> v as u32, None => PREVIOUS_THRESHOLD_DEFAULT},
        "auto_save" => settings.auto_save = match value.as_integer() {Some(v)=> v as u8, None => AUTO_SAVE_INTERVAL_SECONDS},
        "restore_state" => settings.restore_state = value.as_bool().unwrap_or_else(|| true),
        "empty_autoplay" => settings.empty_autoplay = value.as_bool().unwrap_or_else(|| true),
        "accent_color" => settings.accent_color = ACCENT_COLOR,  // TODO: Implement color parsing to TOML
        "collection_paths" => settings.collection_paths = vec![], // TODO: Implement collection paths
        "pixels_per_point" => settings.pixels_per_point = match value.as_float() {Some(v)=> v as f32, None => PIXELS_PER_POINT_DEFAULT},
        _ => warn!("Unknown setting found in TOML: {}", key),
      }
    }
    settings
  }
}

impl Default for Settings {
  fn default() -> Self {
    Self::new()
  }
}

//---------------------------------------------------------------------------------------------------- TESTS
#[cfg(test)]
mod test {
  use std::default::Default;
  use std::path::PathBuf;

  use super::*;

  #[test]
  fn test_partial_settings() {
    let path = PathBuf::from("../assets/festival/gui/state/partial_settings.toml");
    let partial_settings = Settings::read_from_disk(Some(path)).unwrap();

    assert_eq!(partial_settings.artist_sort,        ArtistSort::RuntimeRev);
    assert_eq!(partial_settings.search_sort,        SearchSort::default());
    assert_eq!(partial_settings.album_pixel_size,   227.0);
    assert_eq!(partial_settings.albums_per_row,     6);
  }

  #[test]
  fn empty_settings() {
    let s = Settings::new();
    s.save_atomic().unwrap();
    println!("{:#?}", s.to_string());
  }
}