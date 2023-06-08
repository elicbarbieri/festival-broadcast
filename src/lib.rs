//! # Festival
//! [`Festival`](https://github.com/hinto-janai/festival)'s internals.
//!
//! The crate [`festival`](https://crates.io/crates/festival) is being squatted, so instead, `Festival`'s
//! original name, [`shukusai`](https://crates.io/crates/shukusai), is used.
//!
//! `祝祭/shukusai` translated means: `Festival`.
//!
//! In documentation:
//!
//! - `shukusai` _specifically_ means `Festival`'s internals
//! - `Festival` means a frontend OR the project as a whole
//!
//! ## Warning
//! **The internals are not stable.**
//!
//! **If you're implementing a frontend, you are expected to implement the `Kernel`'s messages correctly.**
//!
//! You can look at [`festival-gui`](https://github.com/hinto-janai/festival/festival-gui)'s code as an example,
//! and the [internal documentation](https://github.com/hinto-janai/festival/src) as reference.
//!
//! ## API
//! The "API" between `shukusai` and the frontends are:
//! - [`kernel::KernelToFrontend`]
//! - [`kernel::FrontendToKernel`]
//!
//! Each frontend must implement the correct message passing behavior to/from the `Kernel` and other various things.
//!
//! `Kernel` itself will handle:
//! - Logging initialization
//! - `Collection` management
//! - Pretty much everything
//!
//! The `Frontend` implementation must:
//! - Keep a channel to `Kernel` open at _all times_
//! - Save and manage its own state/settings
//! - Properly implement the messages `To/From` the `Kernel`
//! - Properly handle shared data
//!
//! ## Shared Data
//! There are shared functions/data that `shukusai` exposes, notably:
//! - [`collection::Collection`] (and everything within it)
//! - [`collection::Key`] (and other keys)
//! - [`audio::AudioState`]
//!
//! It is up to the frontend on how to use these functions/data.
//!
//! A lot of the correct behavior implementation depends on knowledge that _I_ have of the internals.
//! Since _I_ will most likely be creating all the frontends, there are no plans
//! to fully flesh out this documentation for now (it's a lot of work).

//---------------------------------------------------------------------------------------------------- Lints
#![allow(
	clippy::len_zero,
	clippy::type_complexity,
	clippy::module_inception,

	// Should be cleaned up after v1.0.0.
	dead_code,
	unused_variables,
	unused_imports,
)]

#![deny(
	nonstandard_style,
	unused_unsafe,
	unused_mut,
)]

#![forbid(
	future_incompatible,
	let_underscore,
	break_with_label_and_loop,
	coherence_leak_check,
	deprecated,
	duplicate_macro_attributes,
	exported_private_dependencies,
	for_loops_over_fallibles,
	large_assignments,
	overlapping_range_endpoints,
	private_in_public,
	semicolon_in_expressions_from_macros,
	redundant_semicolons,
	unconditional_recursion,
	unreachable_patterns,
	unused_allocation,
	unused_braces,
	unused_comparisons,
	unused_doc_comments,
	unused_parens,
	unused_labels,
	while_true,
	keyword_idents,
	missing_docs,
	non_ascii_idents,
	noop_method_call,
	unreachable_pub,
	single_use_lifetimes,
	variant_size_differences,
)]

#[cfg(not(any(target_pointer_width = "64", target_pointer_width = "32")))]
compile_error!("shukusai is only compatible with 64-bit/32bit CPUs");

#[cfg(not(any(
	target_os = "windows",
	target_os = "macos",
	target_os = "linux",
)))]
compile_error!("shukusai is only compatible with Window/macOS/Linux");

//---------------------------------------------------------------------------------------------------- Private `shukusai` internals.
mod audio;
mod ccd;
mod search;
mod watch;

//---------------------------------------------------------------------------------------------------- Public Re-exports.
pub use readable;
pub use rolock;

//---------------------------------------------------------------------------------------------------- Hidden Re-exports.
#[doc(hidden)]
pub use const_format::assertcp as const_assert;
#[doc(hidden)]
pub use const_format::formatcp as const_format;

//---------------------------------------------------------------------------------------------------- Public `/` stuff.
mod constants;
pub use constants::*;

mod logger;
pub use logger::init_logger;
pub use logger::INIT_INSTANT;

mod thread;
pub use thread::*;

//---------------------------------------------------------------------------------------------------- Public modules.
mod panic;
pub use panic::Panic;

pub use crate::ccd::ImageCache;

/// The main music `Collection` and its inner data
pub mod collection;

/// `Kernel`, the messenger and coordinator
///
/// This is the "API" that all frontends must implement
/// in order to communicate with `Festival`'s internals.
///
/// Your `Frontend` will communicate with `Kernel`, and
/// `Kernel` will talk with the rest of `shukusai`'s internals.
///
/// Messages are sent via `crossbeam::channel`'s with these messages:
/// - [`kernel::KernelToFrontend`]
/// - [`kernel::FrontendToKernel`]
pub mod kernel;

/// Various sorting methods for the `Collection`
///
/// These `enum`'s just represent `Collection` fields and are used for convenience:
/// ```rust,ignore
/// // These two both return the same data.
/// // The enum can be useful when programming frontend stuff.
///
/// collection.album_sort(AlbumSort::ReleaseArtistLexi);
///
/// collection.sort_album_release_artist_lexi;
/// ```
pub mod sort;

/// Audio Signals to `Kernel`
///
/// These are structs that represent files that represent a signal.
///
/// These structs implement `disk::Empty` so that they can easily be created with `touch()`.
///
/// It holds no data but the file existing represents a signal to `Kernel`.
///
/// ## Usage
/// ```rust,ignore
/// Play::touch().unwrap()
/// ```
/// This creates a file with the:
/// - Lowercase struct name in the
/// - `signal` subdirectory of the
/// - `festival` folder which is in the
/// - OS data folder
///
/// Example: `~/.local/share/festival/signal/play`.
///
/// `Kernel` will immediately respond to the signal, in this example,
/// `Kernel` will start audio playback, then delete the file that was created.
pub mod signal;

/// `Frontend`-specific compatibility layers
pub mod frontend;

/// Ancillary `Collection` data validation
///
/// Since the `Collection` uses indices instead of references,
/// it means that there is no lifetime associated with them.
///
/// If a new `Collection` is received, the already existing ancillary data
/// that was pointing to the old one may not be correct, e.g:
/// ```rust,ignore
/// let key = ArtistKey::from(123);
/// assert!(collection.artists.len() > 123);
/// collection.artists[key]; // OK
///
/// let collection = recv_new_collection();
/// collection.artists[key]; // This may or may not panic.
/// ```
/// Even if the key ends up existing, it most likely is pointing at the wrong thing.
///
/// This module provides some common validation methods
/// that checks inputs against an existing `Collection`.
///
/// These functions are used when `Kernel` is loading up the `Collection`
/// and `State` from disk, where there is never a 100% lifetime guarantee between the two.
///
/// These functions are also used for `GUI` configuration settings
/// that hold keys and other misc data like that.
///
/// All methods are free functions that require a `Collection`.
pub mod validate;
