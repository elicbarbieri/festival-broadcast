//---------------------------------------------------------------------------------------------------- Use
use super::{FrontendToKernel, KernelToFrontend};
use crate::{
    audio::{Append, Audio, AudioToKernel, KernelToAudio, Volume},
    ccd::{Ccd, CcdToKernel},
    collection::{AlbumKey, ArtistKey, Collection, SongKey, DUMMY_COLLECTION},
    constants::{AUDIO_VERSION, COLLECTION_VERSION, PLAYLIST_VERSION},
    logger::INIT_INSTANT,
    search::{KernelToSearch, Search, SearchToKernel},
    state::{
        AudioState, AudioStateRestore, Phase, Playlists, AUDIO_STATE, PLAYLISTS, RESETTING,
        RESET_STATE,
    },
    watch::{Watch, WatchToKernel},
};
use benri::{debug_panic, log::*, sync::*, time::*};
use crossbeam::channel::{Receiver, Sender};
use disk::{Bincode2, Json};
use log::{debug, error, info, trace, warn};
use once_cell::sync::Lazy;
use rayon::prelude::*;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use crate::audio::AudioOutputDevice;
#[cfg(feature = "gui")]
use crate::collection::{UNKNOWN_ALBUM, UNKNOWN_ALBUM_ID};
#[cfg(feature = "gui")]
use crate::frontend::gui::{gui_context, gui_request_update};

//---------------------------------------------------------------------------------------------------- Kernel
/// The [`Kernel`] of `Festival`
///
/// [`Kernel`], the messenger and coordinator.
///
/// [`Kernel`] handles all of `Festival`'s internals and acts
/// as a small & simple interface to all the frontends.
///
/// It is highly recommended to read [`festival-gui`](https://github.com/hinto-janai/festival/festival-gui)'s
/// code and [`Festival`'s internal documentation](https://github.com/hinto-janai/festival/src)
/// if you're creating your own frontend for `Festival`.
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
}

// See `GUI`'s Drop impl for reasoning on why this exists.
impl Drop for Kernel {
    fn drop(&mut self) {
        self.exit();
    }
}

// `Kernel` boot process:
//
//`bios()` ---> `boot_loader()` ---> `kernel()` ---> `init()` ---> `userspace()`
//         |                                           |
//         |--- (bios error occurred, skip to init) ---|
//
impl Kernel {
    //-------------------------------------------------- spawn()
    /// [`Kernel`] is started with this.
    ///
    /// For more info, see [here.](https://github.com/hinto-janai/festival/src/kernel)
    ///
    /// [`Kernel`] will return `crossbeam::channel`'s for communication between it and your frontend.
    ///
    /// These channels _should never_ be closed.
    ///
    /// This function itself spawns a new thread for [`Kernel`].
    /// ```rust,ignore
    /// // Don't do this.
    /// std::thread::spawn(|| Kernel::spawn());
    ///
    /// // Do this.
    /// Kernel::spawn();
    /// ```
    ///
    /// The `watch` [`bool`] indicates if `Kernel` should spawn a thread
    /// that watches over the `festival/signal` directory for filesystem-based
    /// [`crate::signal`]'s.
    ///
    /// The `media_controls` [`bool`] indicates if `Kernel` should plug into
    /// the OS and allow communication via the OS-specific media controls.
    pub fn spawn(
        watch: bool,
        media_controls: bool,
    ) -> Result<(Sender<FrontendToKernel>, Receiver<KernelToFrontend>), std::io::Error> {
        // Initialize `RESET_STATE`.
        //		let _ = Lazy::force(&UNKNOWN_ALBUM);

        // Assert `OnceCell`'s were set.
        #[cfg(feature = "gui")]
        {
            let _ = Lazy::force(&UNKNOWN_ALBUM);

            assert!(crate::frontend::gui::GUI_CONTEXT.get().is_some());

            // INVARIANT:
            // `GUI` must not allocate any textures before this.
            //
            // This allocates unknown texture and makes sure it is index `1`.
            let id = UNKNOWN_ALBUM.texture_id(gui_context());
            if id != UNKNOWN_ALBUM_ID {
                panic!("UNKNOWN_ALBUM id: {id:?}, expected: {UNKNOWN_ALBUM_ID:?}");
            }
        }

        // Create `Kernel` <-> `Frontend` channels.
        let (to_frontend, from_kernel) = crossbeam::channel::unbounded::<KernelToFrontend>();
        let (to_kernel, from_frontend) = crossbeam::channel::unbounded::<FrontendToKernel>();

        // Spawn Kernel.
        std::thread::Builder::new()
            .name("Kernel".to_string())
            .stack_size(16_000_000) // 16MB stack.
            .spawn(move || Self::bios(to_frontend, from_frontend, watch, media_controls))?;

        // Return channels.
        Ok((to_kernel, from_kernel))
    }

    //-------------------------------------------------- bios()
    fn bios(
        to_frontend: Sender<KernelToFrontend>,
        from_frontend: Receiver<FrontendToKernel>,
        watch: bool,
        media_controls: bool,
    ) {
        debug!("Kernel Init [1/13] ... entering bios()");

        #[cfg(feature = "panic")]
        // Set panic hook.
        //
        // If `Kernel` or anyone else `panic!()`'s,
        // we want _everyone_ to exit.
        crate::panic::set_panic_hook();

        // Initialize lazy statics.
        let _ = Lazy::force(&DUMMY_COLLECTION);
        let beginning = Lazy::force(&INIT_INSTANT);

        // Create `ResetState`, send to `Frontend`.
        RESET_STATE.write().disk();

        // Attempt to load `Collection` from file.
        debug!("Kernel Init ... Reading Collection{COLLECTION_VERSION} from disk...");
        let now = now!();

        #[cfg(feature = "gui")]
        let collection = Collection::from_versions(&[
            // SAFETY: memmap is used.
            (COLLECTION_VERSION, || unsafe {
                Collection::from_file_memmap()
            }),
            (2, crate::collection::v2::Collection::disk_into),
            (1, crate::collection::v1::Collection::disk_into),
            (0, crate::collection::v0::Collection::disk_into),
        ]);

        #[cfg(feature = "daemon")]
        let collection = Collection::from_versions(&[
            // SAFETY: memmap is used.
            (COLLECTION_VERSION, || unsafe {
                Collection::from_file_memmap()
            }),
        ]);

        match collection {
            // If success, continue to `boot_loader` to convert
            // bytes to actual usable `egui` images.
            Ok((v, collection)) if v == COLLECTION_VERSION => {
                ok_debug!("Kernel Init ... Collection{COLLECTION_VERSION} deserialization ... Took {} seconds", secs_f32!(now));
                Self::boot_loader(
                    collection,
                    to_frontend,
                    from_frontend,
                    *beginning,
                    watch,
                    media_controls,
                    v,
                );
            }
            Ok((v, collection)) => {
                ok_debug!("Kernel Init ... Collection{v} converted to Collection{COLLECTION_VERSION} ... Took {} seconds", secs_f32!(now));
                Self::boot_loader(
                    collection,
                    to_frontend,
                    from_frontend,
                    *beginning,
                    watch,
                    media_controls,
                    v,
                );
            }
            // Else, straight to `init` with default flag set.
            Err(e) => {
                warn!("Kernel Init ... Collection{COLLECTION_VERSION} from file error: {e}");

                // Read `Playlist`'s anyway, and turn all entries into `invalid`.
                // SAFETY: memmap is used.
                let playlists = unsafe { Playlists::from_file_memmap() };
                let playlists = match playlists {
                    Ok(mut p) => {
                        p.all_invalid();
                        Some(p)
                    }
                    Err(_) => None,
                };

                Self::init(
                    None,
                    None,
                    playlists,
                    to_frontend,
                    from_frontend,
                    *beginning,
                    watch,
                    media_controls,
                );
            }
        }
    }

    //-------------------------------------------------- boot_loader()
    fn boot_loader(
        collection: Collection,
        to_frontend: Sender<KernelToFrontend>,
        from_frontend: Receiver<FrontendToKernel>,
        beginning: std::time::Instant,
        watch: bool,
        media_controls: bool,
        version: u8,
    ) {
        debug!("Kernel Init [2/13] ... entering boot_loader()");

        // If the `Collection` got upgraded, that means
        // we need to save the new version to disk.
        if COLLECTION_VERSION != version {
            let now = now!();

            debug!("Kernel ... Collection{version} != Collection{COLLECTION_VERSION}, saving to disk...");

            match unsafe { collection.save_atomic_memmap() } {
                Ok(md) => debug!("Kernel ... Collection{COLLECTION_VERSION}: {md}"),
                Err(e) => {
                    debug_panic!("Kernel ... Collection{COLLECTION_VERSION}: {e}");
                    fail!("Kernel ... Collection{COLLECTION_VERSION}: {e}");
                }
            }

            debug!(
                "Kernel ... Collection{COLLECTION_VERSION} save, took {} seconds",
                secs_f32!(now)
            );
        }

        #[cfg(feature = "gui")]
        let from_ccd = {
            // We successfully loaded `Collection`.
            // Create `CCD` channel + thread and make it convert images.
            debug!("Kernel Init [3/13] ... spawning CCD");
            let (ccd_send, from_ccd) = crossbeam::channel::unbounded::<CcdToKernel>();
            if let Err(e) = std::thread::Builder::new()
                .name("CCD".to_string())
                .spawn(move || Ccd::convert_art(ccd_send, collection))
            {
                panic!("Kernel Init [3/13] ... failed to spawn CCD: {e}");
            }
            from_ccd
        };

        #[cfg(not(feature = "gui"))]
        debug!("Kernel Init [3/13] ... skipping CCD");

        // Before hanging on `CCD`, read `AudioState` file.
        // Note: This is a `Result`.
        debug!("Kernel Init [4/13] ... reading AudioState");
        // SAFETY: memmap is used.
        let state = unsafe { AudioState::from_file_memmap() };

        // Before hanging on `CCD`, read `Playlists` file.
        // Note: This is a `Result`.
        debug!("Kernel Init [5/13] ... reading Playlists");
        // SAFETY: memmap is used.
        let playlists = unsafe { Playlists::from_file_memmap() };

        // Set `ResetState` to `Start` + `Art` phase.
        {
            let mut lock = RESET_STATE.write();
            lock.start();
            lock.phase = Phase::Art;
        }

        #[cfg(feature = "gui")]
        let collection = {
            // Wait for `Collection` to be returned by `CCD`.
            debug!("Kernel Init [6/13] ... waiting on CCD");

            loop {
                use CcdToKernel::*;
                match recv!(from_ccd) {
                    // We received an incremental update.
                    // Update the current `ResetState` values to match.
                    UpdateIncrement((increment, specific)) => {
                        RESET_STATE.write().new_increment(increment, specific)
                    }

                    // We're onto the next phase in `Collection` creation process.
                    // Update the current `ResetState` values to match.
                    UpdatePhase((percent, phase)) => RESET_STATE.write().new_phase(percent, phase),

                    // `CCD` was successful. We got the new `Collection`.
                    NewCollection(collection) => break collection,
                }
            }
        };

        #[cfg(not(feature = "gui"))]
        let collection = {
            debug!("Kernel Init [6/13] ... skipping CCD");
            Arc::new(collection)
        };

        // We're done with `CCD`.
        RESET_STATE.write().done();

        // Continue to `kernel` to verify data.
        Self::kernel(
            collection,
            state,
            playlists,
            to_frontend,
            from_frontend,
            beginning,
            watch,
            media_controls,
        );
    }

    //-------------------------------------------------- kernel()
    fn kernel(
        collection: Arc<Collection>,
        audio: Result<AudioState, anyhow::Error>,
        playlists: Result<Playlists, anyhow::Error>,
        to_frontend: Sender<KernelToFrontend>,
        from_frontend: Receiver<FrontendToKernel>,
        beginning: std::time::Instant,
        watch: bool,
        media_controls: bool,
    ) {
        debug!("Kernel Init [7/13] ... entering kernel()");
        let mut audio = match audio {
            Ok(s) => {
                info!("Kernel Init ... AudioState{AUDIO_VERSION} from disk");
                s
            }
            Err(e) => {
                warn!("Kernel Init ... AudioState failed from disk: {e}, returning default AudioState{AUDIO_VERSION}");
                AudioState::new()
            }
        };

        // Check if `AUDIO_STATE`'s `SongKey` is valid.
        if !crate::validate::song(&collection, audio.song.unwrap_or_else(SongKey::zero)) {
            info!("AudioState ... SongKey invalid, resetting to None");
            audio.song = None;
        }

        // Check if `AUDIO_STATE` indices into itself are in-bounds.
        if let Some(idx) = audio.queue_idx {
            if audio.queue.get(idx).is_none() {
                info!("AudioState ... Queue index invalid, resetting to None");
                audio.queue_idx = None;
            }
        }

        // Check if all of `AUDIO_STATE`'s queue keys are valid.
        if !crate::validate::song(
            &collection,
            audio.queue.iter().max().unwrap_or(&SongKey::zero()),
        ) {
            info!("AudioState ... Queue contains SongKey that is out-of-bounds, clear()'ing");
            audio.queue.clear();
        }

        // Check playlist validity.
        let playlists = if let Ok(mut playlists) = playlists {
            playlists.validate(&collection);
            Some(playlists)
        } else {
            None
        };

        Self::init(
            Some(collection),
            Some(audio),
            playlists,
            to_frontend,
            from_frontend,
            beginning,
            watch,
            media_controls,
        );
    }

    //-------------------------------------------------- init()
    fn init(
        collection: Option<Arc<Collection>>,
        audio: Option<AudioState>,
        playlists: Option<Playlists>,
        to_frontend: Sender<KernelToFrontend>,
        from_frontend: Receiver<FrontendToKernel>,
        beginning: std::time::Instant,
        watch: bool,
        media_controls: bool,
    ) {
        debug!("Kernel Init ... entering init()");

        // Handle potentially missing `Collection`.
        let collection = match collection {
            Some(c) => {
                debug!("Kernel Init [8/13] ... Collection found");
                c
            }
            None => {
                debug!("Kernel Init [8/13] ... Collection NOT found, returning default");
                Arc::new(Collection::new())
            }
        };

        // Handle potentially missing `AudioState`.
        let audio = match audio {
            Some(a) => {
                debug!("Kernel Init [9/13] ... AudioState found");
                a
            }
            None => {
                debug!("Kernel Init [9/13] ... AudioState NOT found, returning default");
                AudioState::new()
            }
        };

        // Handle potentially missing `Playlists`.
        match playlists {
            Some(p) => {
                debug!("Kernel Init [10/13] ... Playlists found");
                *PLAYLISTS.write() = p;
            }
            None => debug!("Kernel Init [10/13] ... Playlists NOT found"),
        };

        // Send `Collection` to `Frontend`.
        send!(
            to_frontend,
            KernelToFrontend::NewCollection(Arc::clone(&collection))
        );
        #[cfg(feature = "gui")]
        gui_request_update();

        // Create `To` channels.
        let (to_search, search_recv) = crossbeam::channel::unbounded::<KernelToSearch>();
        let (to_audio, audio_recv) = crossbeam::channel::unbounded::<KernelToAudio>();

        // Create `From` channels.
        let (search_send, from_search) = crossbeam::channel::unbounded::<SearchToKernel>();
        let (audio_send, from_audio) = crossbeam::channel::unbounded::<AudioToKernel>();
        let (watch_send, from_watch) = crossbeam::channel::unbounded::<WatchToKernel>();

        // Create `Kernel`.
        let kernel = Self {
            // Channels.
            to_frontend,
            from_frontend,
            to_search,
            from_search,
            to_audio,
            from_audio,
            from_watch,

            // Data.
            collection,
        };

        // Spawn `Audio`.
        let collection = Arc::clone(&kernel.collection);
        match std::thread::Builder::new()
            .name("Audio".to_string())
            .spawn(move || {
                Audio::init(
                    AudioOutputDevice::default(),
                    collection,
                    audio,
                    audio_send,
                    audio_recv,
                    media_controls
                );
            }) {
            Ok(_) => debug!("Kernel Init [11/13] ... spawned Audio"),
            Err(e) => panic!("Kernel Init [11/13] ... failed to spawn Audio: {e}"),
        }

        // Spawn `Search`.
        let collection = Arc::clone(&kernel.collection);
        match std::thread::Builder::new()
            .name("Search".to_string())
            .spawn(move || Search::init(collection, search_send, search_recv))
        {
            Ok(_) => debug!("Kernel Init [12/13] ... spawned Search"),
            Err(e) => panic!("Kernel Init [12/13] ... failed to spawn Search: {e}"),
        }

        // Spawn `Watch`.
        if watch {
            match std::thread::Builder::new()
                .name("Watch".to_string())
                .spawn(move || Watch::init(watch_send))
            {
                Ok(_) => debug!("Kernel Init [13/13] ... spawned Watch"),
                Err(e) => fail!("Kernel Init [13/13] ... failed to spawn Watch: {e}"),
            }
        } else {
            debug!("Kernel Init [13/13] ... skipping Watch");
        }

        // We're done, enter main `userspace` loop.
        debug!(
            "Kernel Init ... entering userspace(), took {} seconds",
            secs_f32!(beginning)
        );
        Self::userspace(kernel);
    }
}

//---------------------------------------------------------------------------------------------------- Main Kernel loop (userspace)
impl Kernel {
    fn userspace(mut self) {
        ok_debug!("Kernel");
        // Array of our channels we can `select` from.
        let mut select = crossbeam::channel::Select::new();
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

        assert_eq!(0, select.recv(&frontend));
        assert_eq!(1, select.recv(&search));
        assert_eq!(2, select.recv(&audio));
        assert_eq!(3, select.recv(&watch));

        // 1) Hang until message is ready.
        // 2) Receive the message and pass to appropriate function.
        // 3) Loop.
        loop {
            match select.ready() {
                0 => self.recv_from_frontend(recv!(self.from_frontend)),
                1 => self.recv_from_search(recv!(self.from_search)),
                2 => self.recv_from_audio(recv!(self.from_audio)),
                3 => self.recv_from_watch(recv!(self.from_watch)),
                _ => {
                    error!("Kernel - Received an unknown message");
                    debug_panic!("Kernel - Received an unknown message");
                }
            }
        }
    }

    //-------------------------------------------------- Message handling.
    #[inline(always)]
    // We got a message from `GUI`.
    fn recv_from_frontend(&mut self, msg: FrontendToKernel) {
        use crate::kernel::FrontendToKernel::*;
        match msg {
            // Audio playback.
            Toggle => send!(self.to_audio, KernelToAudio::Toggle),
            Play => send!(self.to_audio, KernelToAudio::Play),
            Pause => send!(self.to_audio, KernelToAudio::Pause),
            Next => send!(self.to_audio, KernelToAudio::Next),
            Previous(threshold) => send!(self.to_audio, KernelToAudio::Previous(threshold)),
            Stop => send!(self.to_audio, KernelToAudio::Clear(false)),
            // Audio settings.
            Repeat(r) => send!(self.to_audio, KernelToAudio::Repeat(r)),
            Volume(volume) => send!(self.to_audio, KernelToAudio::Volume(volume.check())),
            Seek(tuple) => send!(self.to_audio, KernelToAudio::Seek(tuple)),

            // Queue.
            QueueAddSong(tuple) => send!(self.to_audio, KernelToAudio::QueueAddSong(tuple)),
            QueueAddAlbum(tuple) => send!(self.to_audio, KernelToAudio::QueueAddAlbum(tuple)),
            QueueAddArtist(tuple) => send!(self.to_audio, KernelToAudio::QueueAddArtist(tuple)),
            QueueAddPlaylist(tuple) => send!(self.to_audio, KernelToAudio::QueueAddPlaylist(tuple)),
            Shuffle => send!(self.to_audio, KernelToAudio::Shuffle),
            Clear(play) => send!(self.to_audio, KernelToAudio::Clear(play)),
            Skip(num) => send!(self.to_audio, KernelToAudio::Skip(num)),
            Back(num) => send!(self.to_audio, KernelToAudio::Back(num)),

            // Queue Index.
            QueueSetIndex(q_key) => send!(self.to_audio, KernelToAudio::QueueSetIndex(q_key)),
            QueueRemoveRange(tuple) => send!(self.to_audio, KernelToAudio::QueueRemoveRange(tuple)),

            // Audio State.
            RestoreAudioState => send!(self.to_audio, KernelToAudio::RestoreAudioState),
            SetOutputDevice(device) => send!(self.to_audio, KernelToAudio::SetOutputDevice(device)),

            // Collection.
            NewCollection(paths) => self.ccd_mode(paths),
            CachePath(paths) => Self::cache_path(paths),
            Search(string) => send!(self.to_search, KernelToSearch::Search(string)),

            // Exit.
            Exit => self.exit(),
        }
    }

    #[inline(always)]
    // We got a message from `Search`.
    fn recv_from_search(&self, msg: SearchToKernel) {
        use crate::search::SearchToKernel::*;
        match msg {
            Resp(keychain) => send!(self.to_frontend, KernelToFrontend::SearchResp(keychain)),
        }
    }

    #[inline(always)]
    // We got a message from `Audio`.
    fn recv_from_audio(&self, msg: AudioToKernel) {
        use crate::audio::AudioToKernel::*;
        match msg {
            DeviceError(string) => send!(
                self.to_frontend,
                KernelToFrontend::DeviceError(string.to_string())
            ),
            PlayError(string) => send!(
                self.to_frontend,
                KernelToFrontend::PlayError(string.to_string())
            ),
            SeekError(string) => send!(
                self.to_frontend,
                KernelToFrontend::SeekError(string.to_string())
            ),
            PathError((song_key, string)) => send!(
                self.to_frontend,
                KernelToFrontend::PathError((song_key, string.to_string()))
            ),
        }
    }

    #[inline(always)]
    // We got a message from `Watch`.
    fn recv_from_watch(&self, msg: WatchToKernel) {
        use crate::audio::{Repeat, Seek};
        use crate::watch::WatchToKernel::*;
        match msg {
            Toggle => send!(self.to_audio, KernelToAudio::Toggle),
            Play => send!(self.to_audio, KernelToAudio::Play),
            Pause => send!(self.to_audio, KernelToAudio::Pause),
            Next => send!(self.to_audio, KernelToAudio::Next),
            Previous => send!(self.to_audio, KernelToAudio::Previous(None)),
            Stop => send!(self.to_audio, KernelToAudio::Clear(false)),
            Shuffle => send!(self.to_audio, KernelToAudio::Shuffle),
            RepeatSong => send!(self.to_audio, KernelToAudio::Repeat(Repeat::Song)),
            RepeatQueue => send!(self.to_audio, KernelToAudio::Repeat(Repeat::Queue)),
            RepeatOff => send!(self.to_audio, KernelToAudio::Repeat(Repeat::Off)),

            // Content signals.
            Volume(v) => send!(self.to_audio, KernelToAudio::Volume(v.check())),
            Clear(b) => send!(self.to_audio, KernelToAudio::Clear(b)),
            Seek(s) => send!(self.to_audio, KernelToAudio::Seek((Seek::Absolute, s))),
            SeekForward(s) => send!(self.to_audio, KernelToAudio::Seek((Seek::Forward, s))),
            SeekBackward(s) => send!(self.to_audio, KernelToAudio::Seek((Seek::Backward, s))),
            Index(s) => send!(self.to_audio, KernelToAudio::QueueSetIndex(s)),
            Skip(s) => send!(self.to_audio, KernelToAudio::Skip(s)),
            Back(s) => send!(self.to_audio, KernelToAudio::Back(s)),
            //			ArtistKey(k)    => send!(self.to_audio, KernelToAudio::ArtistKey(k)),
            //			AlbumKey(k)     => send!(self.to_audio, KernelToAudio::AlbumKey(k)),
            //			SongKey(k)      => send!(self.to_audio, KernelToAudio::SongKey(k)),
            //			Artist(s)       => send!(self.to_audio, KernelToAudio::Artist(k)),
            //			Album(s)        => send!(self.to_audio, KernelToAudio::Album(k)),
            //			Song(s)         => send!(self.to_audio, KernelToAudio::Song(k)),
        }
    }

    //-------------------------------------------------- Misc message handling.
    // The `Frontend` is exiting, save everything.
    fn exit(&mut self) -> ! {
        let mut err = None::<String>;

        // Set the saved state's volume
        // to the correct global.
        let volume = Volume::new(atomic_load!(crate::state::VOLUME));

        {
            let mut state = AUDIO_STATE.write();
            state.volume = volume;

            // Save `AudioState`.
            match state.save_atomic() {
                Ok(o) => ok!("Kernel - AudioState{AUDIO_VERSION} save: {o}"),
                Err(e) => {
                    fail!("Kernel - AudioState{AUDIO_VERSION} save: {e}");
                    err = Some(e.to_string());
                }
            }
        }

        // Save `Playlists`.
        match PLAYLISTS.read().save_atomic() {
            Ok(o) => ok!("Kernel - Playlists{PLAYLIST_VERSION} save: {o}"),
            Err(e) => {
                fail!("Kernel - Playlists{PLAYLIST_VERSION} save: {e}");
                err = Some(e.to_string());
            }
        }

        if let Some(err) = err {
            send!(self.to_frontend, KernelToFrontend::Exit(Err(err)));
        } else {
            send!(self.to_frontend, KernelToFrontend::Exit(Ok(())));
        }

        // Hang forever.
        info!(
            "Kernel - Total uptime: {}",
            readable::Time::from(*crate::logger::INIT_INSTANT)
        );
        loop {
            std::thread::park();
        }
    }

    //-------------------------------------------------- CachePath.
    // A separate thread is responsible for walking these
    // directories since `Kernel` really shouldn't be blocked
    // doing work at any given moment.
    #[inline(always)]
    fn cache_path(mut paths: Vec<PathBuf>) {
        let now = now!();
        debug!("Kernel - Starting CachePath...");

        let cache_path = std::thread::Builder::new().name("CachePath".to_string());

        if let Err(e) = cache_path.spawn(move || {
            paths.retain(|p| p.exists());
            paths.par_sort();
            paths.dedup();

            for path in paths
                .into_iter()
                .flat_map(|p| walkdir::WalkDir::new(p).follow_links(true))
                .filter_map(Result::ok)
                .map(walkdir::DirEntry::into_path)
            {
                // If we're resetting the `Collection`, we might be doing
                // more harm by thrashing the filesystem, so just exit.
                if atomic_load!(RESETTING) {
                    debug!("CachePath - CCD detected, exiting early");
                    return;
                }

                trace!("CachePath - {path:?}");
                infer::get_from_path(&path);
            }

            debug!("CachePath - took {} seconds, bye!", secs_f32!(now));
        }) {
            panic!("Kernel ... failed to spawn CachePath: {e}");
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
        atomic_store!(RESETTING, true);

        // Set our `ResetState`.
        RESET_STATE.write().start();

        // INVARIANT:
        // `GUI` is expected to drop its pointer by itself
        // after requesting the new `Collection`.
        //
        // Drop your pointers.
        send!(self.to_search, KernelToSearch::DropCollection);
        send!(self.to_audio, KernelToAudio::DropCollection);

        // Create `CCD` channel.
        let (ccd_send, from_ccd) = crossbeam::channel::unbounded::<CcdToKernel>();

        // Convert our current `AudioState` to string keys.
        let audio_state_restore =
            AudioStateRestore::from_audio_state(&AUDIO_STATE.read(), &self.collection);

        // Give the last ownership of the
        // old `Collection` pointer to `CCD`.
        //
        // Since other parts of the system use
        // `collection = Collection::dummy()` to "drop"
        // the `Collection`, giving the dummy as the "old"
        // `Collection` to `CCD` won't work, it'll hang forever.
        // So, if our "old" `Collection` is still the dummy,
        // create a new private one here.
        let dummy = Collection::dummy();
        let old_collection = if self.collection == dummy {
            debug!("Kernel - Old Collection is dummy, creating new private one...");
            Arc::new(Collection::new())
        } else {
            Arc::clone(&self.collection)
        };
        self.collection = Collection::dummy();

        // If there is another `CCD` still alive
        // saving, wait for it to finish.
        if crate::state::saving() {
            // Set `ResetState` to `Wait` phase.
            RESET_STATE.write().wait();

            while crate::state::saving() {
                info!("Kernel - Another CCD is still saving, waiting...");
                benri::sleep_millis!(500);
            }
        }

        // Set `ResetState` to `Start` phase.
        RESET_STATE.write().start();

        // If the provided PATHs is empty,
        // scan the default Music directory.
        let paths = match paths.is_empty() {
            false => paths,
            true => match dirs::audio_dir() {
                Some(p) => vec![p],
                None => vec![],
            },
        };

        // Spawn `CCD`.
        if let Err(e) = std::thread::Builder::new()
            .name("CCD".to_string())
            .stack_size(16_000_000) // 16MB stack.
            .spawn(move || Ccd::new_collection(ccd_send, old_collection, paths))
        {
            panic!("Kernel - failed to spawn CCD: {e}");
        }

        // Listen to `CCD`.
        let collection = loop {
            use crate::ccd::CcdToKernel::*;

            // What message did `CCD` send?
            match recv!(from_ccd) {
                // We received an incremental update.
                // Update the current `KernelState.ResetState` values to match.
                UpdateIncrement((increment, specific)) => {
                    RESET_STATE.write().new_increment(increment, specific)
                }

                // We're onto the next phase in `Collection` creation process.
                // Update the current `ResetState` values to match.
                UpdatePhase((percent, phase)) => RESET_STATE.write().new_phase(percent, phase),

                // `CCD` was successful. We got the new `Collection`.
                NewCollection(collection) => break collection,
            }
        };

        self.collection = collection;

        // Attempt to restore audio state.
        // INVARIANT:
        // We must set `AUDIO_STATE` in a valid state
        // before sending the `Collection` to `Audio`, as that
        // will trigger it to assume keys and `AUDIO_STATE` are valid.
        let audio_state = audio_state_restore.into_audio_state(&self.collection);
        *AUDIO_STATE.write() = audio_state;

        // Send new pointers to everyone.
        send!(
            self.to_audio,
            KernelToAudio::NewCollection(Arc::clone(&self.collection))
        );
        send!(
            self.to_search,
            KernelToSearch::NewCollection(Arc::clone(&self.collection))
        );
        send!(
            self.to_frontend,
            KernelToFrontend::NewCollection(Arc::clone(&self.collection))
        );

        #[cfg(feature = "gui")]
        gui_request_update();

        // Set our `ResetState`, we're done.
        RESET_STATE.write().done();
        atomic_store!(RESETTING, false);
    }
}

//---------------------------------------------------------------------------------------------------- TESTS
//#[cfg(test)]
//mod tests {
//  #[test]
//  fn __TEST__() {
//  }
//}
