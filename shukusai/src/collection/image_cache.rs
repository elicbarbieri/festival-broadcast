//---------------------------------------------------------------------------------------------------- Use
use disk::Plain;
use crate::constants::{
	FESTIVAL,
	FRONTEND_SUB_DIR,
	IMAGE_CACHE_SUB_DIR,
};
use crate::collection::Collection;
use serde::{Serialize,Deserialize};
use const_format::formatcp;

//---------------------------------------------------------------------------------------------------- __NAME__
disk::plain!(ImageCache, disk::Dir::Cache, FESTIVAL, formatcp!("{FRONTEND_SUB_DIR}/{IMAGE_CACHE_SUB_DIR}"), "timestamp.txt");
#[derive(Copy,Clone,Debug,Default,PartialEq,PartialOrd,Serialize,Deserialize)]
/// File representing cached images of the `Collection`.
///
/// This file holds the timestamp of the `Collection` the images in the same directory refer too.
///
/// This gets written in the OS cache folder, within `image/` as `timestamp.txt`.
///
/// Some other parts of Festival require a hard PATH
/// to an image file to display it (`GUI` with `souvlaki`).
///
/// So, `CCD` will write all the images contained in a newly created `Collection`
/// to this cache location as individual files. The name of the files is just
/// `AlbumKey.jpg` where `AlbumKey` is the actual internal index, e.g, if we
/// were saving `AlbumKey(123)`, that album's art would be saved as `123.jpg`.
///
/// This is optional and it isn't the end of the world if we don't have these images.
pub struct ImageCache(pub u64);
