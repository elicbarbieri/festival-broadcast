//---------------------------------------------------------------------------------------------------- Use
use crate::{
    audio::{
        output::AudioOutput, device::AudioOutputDevice, Append, AudioToKernel,
        KernelToAudio, Repeat, Seek, Volume,
    },
    collection::{AlbumKey, ArtistKey, Collection, SongKey},
    state::{AudioState, AUDIO_STATE, MEDIA_CONTROLS_RAISE, MEDIA_CONTROLS_SHOULD_EXIT, VOLUME},
};
use anyhow::anyhow;
use benri::{debug_panic, flip, log::*, sync::*};
use crossbeam::channel::{Receiver, Sender};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;

use readable::Runtime;
use std::fs::File;
use std::sync::atomic::AtomicU32;
use std::time::Duration;
use symphonia::core::{
    audio::{AsAudioBufferRef, AudioBuffer, Signal},
    codecs::{Decoder, DecoderOptions},
    formats::{FormatOptions, FormatReader},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
    units::{Time, TimeBase},
};
use cpal::Device;

#[cfg(feature = "gui")]
use crate::frontend::gui::gui_request_update;

//---------------------------------------------------------------------------------------------------- Constants

// In the audio packet demux/decode loop, how much time
// should we spare to listen for `Kernel` messages?
//
// If too high, we won't process the audio
// fast enough and it will end up choppy.
//
// If too low, we will be spinning the CPU more.
const RECV_TIMEOUT: Duration = Duration::from_millis(4);

// If there are multiple messages in the queue, how
// many additional ones should we process before continuing on?
//
// This shouldn't be too high, otherwise we'll be
// delaying the audio demux/decode code and audio
// will be choppy.
//
// These messages don't wait additional time with `RECV_TIMEOUT`,
// they either are there or we break and continue with audio.
const MSG_PROCESS_LIMIT: u8 = 6;

/// When receiving a `Previous` signal, there is runtime
/// threshold for the song to reach until we reset the
/// current instead of actually going to the previous song.
///
/// If `None` is passed in the `Previous` signal,
/// this atomic is used instead.
///
/// A `Frontend` can mutate this data, and it will also affect
/// the default threshold used by the media controls.
pub static PREVIOUS_THRESHOLD: AtomicU32 = AtomicU32::new(PREVIOUS_THRESHOLD_DEFAULT);

/// The default used in [`PREVIOUS_THRESHOLD`].
pub const PREVIOUS_THRESHOLD_DEFAULT: u32 = 3;

//---------------------------------------------------------------------------------------------------- Audio Init
pub(crate) struct Audio {
    // A handle to the audio output device.
    output: AudioOutput,

    // SOMEDAY:
    // We should probably have a `next` AudioReader lined up and ready to go.
    // There is currently no audible gap between `set()`'s but that may not
    // be the case on a _really_ slow HDD.
    //
    // The audio sample buffer we hold may be enough to mask this, though.
    //
    // Also, straying away from the current `cursor + queue` design and
    // having a `next` thing ready makes things more complicated, although,
    // when the time comes to rewrite all of `Audio` and `AudioState` and
    // not tangle them together so much, this should be considered.

    // The current song.
    current: Option<AudioReader>,

    // The existence of this field means we should
    // be seeking in the next loop iteration.
    seek: Option<Time>,

    // A local copy of `AUDIO_STATE`.
    // This exists so we don't have to lock
    // in the loop every time we want to read values.
    //
    // It will get updated when we receive
    // a message, e.g, to change the volume.
    state: AudioState,

    // OS media controls.
    //
    // This is optional for the user, but also
    // it's `Option` because it might fail.
    media_controls: Option<souvlaki::MediaControls>,
    // HACK:
    // This always exists because of `&` not living
    // long enough when using `Select` in the main loop.
    from_mc: Receiver<souvlaki::MediaControlEvent>,

    collection: Arc<Collection>,          // Pointer to `Collection`
    to_kernel: Sender<AudioToKernel>,     // Channel TO `Kernel`
    from_kernel: Receiver<KernelToAudio>, // Channel FROM `Kernel`
}

// This is a container for some
// metadata needed to be held while
// playing back a song.
pub(crate) struct AudioReader {
    // The current song.
    reader: Box<dyn FormatReader>,
    // The current song's decoder.
    decoder: Box<dyn Decoder>,
    // Song's `TimeBase`
    timebase: TimeBase,
    // Elapsed `Time`
    time: Time,
}

impl Audio {
    #[inline(always)]
    // Kernel starts `Audio` with this.
    pub(crate) fn init(
        output_device: AudioOutputDevice,
        collection: Arc<Collection>,
        state: AudioState,
        to_kernel: Sender<AudioToKernel>,
        from_kernel: Receiver<KernelToAudio>,
        media_controls: bool,
    ) {
        trace!("Audio Init - State:\n{state:#?}");

        // To prevent holding onto this `Collection`
        // and preventing CCD from continuing in the case
        // that we continue to `loop` in the next block,
        // turn this `Arc` into `Weak`.
        let weak = Arc::downgrade(&collection);
        drop(collection);

        // Loop until we can open the audio output device.
        // Use default SignalSpec.  If decoded audio is at a different sample rate,
        // the audio output device will be re-opened with the correct SignalSpec.
        let output = match AudioOutput::try_open(output_device, None, symphonia::core::units::Duration::from(4_608_u64)) {
            Ok(o) => o,
            Err(e) => {
                fail!("Audio Init [1/3] ... Cannot create output device");
                return;
            }
        };

        // Media Controls.
        let (to_audio, from_mc) = crossbeam::channel::unbounded::<souvlaki::MediaControlEvent>();
        let media_controls = if media_controls {
            match crate::audio::media_controls::init_media_controls(to_audio) {
                Ok(mc) => {
                    debug!("Audio Init [2/3] ... media controls");
                    Some(mc)
                }
                Err(e) => {
                    warn!("Audio Init [2/3] ... media controls failed: {e}");
                    None
                }
            }
        } else {
            debug!("Audio Init [2/3] ... skipping media controls");
            None
        };

        // We're ready, attempt to turn `Weak` -> `Arc`.
        let collection = match std::sync::Weak::upgrade(&weak) {
            Some(arc) => arc,
            None => Collection::dummy(),
        };

        // Init data.
        let audio = Self {
            output,
            current: None,
            seek: None,
            state,
            media_controls,
            from_mc,
            collection,
            to_kernel,
            from_kernel,
        };

        // Set real-time thread priority.
        //
        // TODO: https://github.com/mozilla/audio_thread_priority/pull/23
        //
        // The sample rate be set to `96_000` eventually
        // but anything higher than `85_880` will panic.
        match audio_thread_priority::promote_current_thread_to_real_time(
            512,      // audio buffer frames (0 == default sensible value)
            44_100, // audio sample rate (assume 44.1kHz)
        ) {
            Ok(_) => ok_debug!("Audio Init [3/3] ... realtime priority"),
            Err(_) => fail!("Audio Init [3/3] ... realtime priority"),
        }

        // Start `main()`.
        Self::main(audio);
    }
}

//---------------------------------------------------------------------------------------------------- Main Audio loop.
impl Audio {
    //-------------------------------------------------- The "main" loop.
    #[inline(always)]
    fn main(mut self) {
        ok_debug!("Audio");

        // Array of our channels we can `select` from.
        //
        // `Kernel` == `[0]`
        // `Mc`     == `[1]`
        let mut select = crossbeam::channel::Select::new();
        let kernel = self.from_kernel.clone();
        let mc = self.from_mc.clone();

        assert_eq!(0, select.recv(&kernel));
        assert_eq!(1, select.recv(&mc));

        loop {
            //------ Kernel message.
            // If we're playing something, only listen for messages for a few millis.
            if self.state.playing {
                match select.try_ready() {
                    // Kernel.
                    Ok(0) => {
                        self.kernel_msg(recv!(self.from_kernel));
                        // If there's more messages, process them before continuing on.
                        for _ in 0..MSG_PROCESS_LIMIT {
                            if let Ok(msg) = self.from_kernel.try_recv() {
                                self.kernel_msg(msg);
                            } else {
                                break;
                            }
                        }
                    }
                    // Mc.
                    Ok(1) => {
                        if self.media_controls.is_some() {
                            self.mc_msg(recv!(self.from_mc));
                        }
                    }
                    _ => {}
                }
            // Else, sleep on `Kernel`.
            } else {
                // Flush output on pause.
                //
                // These prevents a few stutters from happening
                // when users switch songs (the leftover samples
                // get played then and sound terrible).
                trace!("Audio - Pause [1/3]: flush()'ing leftover samples");
                self.output.flush();

                trace!("Audio - Pause [2/3]: waiting on message...");
                match select.ready() {
                    0 => {
                        trace!("Audio - Pause [3/3]: woke up from Kernel message...!");
                        self.kernel_msg(recv!(self.from_kernel));
                    }

                    1 => {
                        trace!("Audio - Pause [3/3]: woke up from MediaControls message...!");
                        self.mc_msg(recv!(self.from_mc));
                    }

                    _ => unreachable!(),
                }
            }

            //------ Audio decoding & demuxing.
            if let Some(audio_reader) = &mut self.current {
                let AudioReader {
                    reader,
                    decoder,
                    timebase,
                    time,
                } = audio_reader;

                //------ Audio seeking.
                if let Some(seek) = self.seek.take() {
                    // Seeking a little bit before the requested
                    // prevents some stuttering.
                    if let Err(e) = reader.seek(
                        symphonia::core::formats::SeekMode::Coarse,
                        symphonia::core::formats::SeekTo::Time {
                            time: seek,
                            track_id: None,
                        },
                    ) {
                        send!(self.to_kernel, AudioToKernel::SeekError(anyhow!(e)));
                    } else {
                        AUDIO_STATE.write().elapsed = Runtime::from(seek.seconds);
                        #[cfg(feature = "gui")]
                        gui_request_update();
                    }
                }

                // If the above message didn't set `playing` state, that
                // means we don't need to continue below to decode audio.
                if !self.state.playing {
                    continue;
                }

                // Decode and play the packets belonging to the selected track.
                // Get the next packet from the format reader.
                let packet = match reader.next_packet() {
                    Ok(packet) => packet,
                    // We're done playing audio.
                    // This "end of stream" error is currently the only way
                    // a FormatReader can indicate the media is complete.
                    Err(symphonia::core::errors::Error::IoError(_err)) => {
                        self.skip(1, &mut AUDIO_STATE.write());
                        #[cfg(feature = "gui")]
                        gui_request_update();
                        continue;
                    }
                    Err(err) => {
                        error!("Audio - {err}");
                        continue;
                    }
                };

                // Decode the packet into audio samples.
                match decoder.decode(&packet) {
                    Ok(decoded) => {
                        // Get the audio buffer specification. This is a description of the decoded
                        // audio buffer's sample format and sample rate.
                        let spec = *decoded.spec();

                        // Get the capacity of the decoded buffer. Note that this is capacity, not
                        // length! The capacity of the decoded buffer is constant for the life of the
                        // decoder, but the length is not.
                        let duration = decoded.capacity() as u64;

                        if spec != self.output.spec || duration != self.output.duration {
                            match self.output.reopen_stream(&spec, &duration) {
                                Ok(_) => self.output.flush(),
                                Err(e) => {
                                    self.state.playing = false;
                                    AUDIO_STATE.write().playing = false;
                                    send!(self.to_kernel, AudioToKernel::DeviceError(e.into_anyhow()));
                                    continue;
                                }
                            }
                        }

                        // Write to audio output device.
                        //
                        // This call blocks for as long as it takes
                        // for us to write the buffer to the audio
                        // device/server.
                        //
                        // If there already is a buffer of audio, this means
                        // we'll have to wait - around 0.05-0.08~ seconds per buffer.
                        //
                        // Resampling + Volume is applied within `write()`.
                        if let Err(e) = self.output.write(decoded) {
                            // Pause playback on write error.
                            self.state.playing = false;
                            AUDIO_STATE.write().playing = false;
                            send!(self.to_kernel, AudioToKernel::PlayError(e.into_anyhow()));
                            continue;
                        }

                        // Set runtime timestamp.
                        let new_time = timebase.calc_time(packet.ts);
                        if time.seconds != new_time.seconds {
                            *time = new_time;

                            // Set state.
                            AUDIO_STATE.write().elapsed = Runtime::from(time.seconds);

                            // Wake up the GUI thread.
                            #[cfg(feature = "gui")]
                            gui_request_update();

                            // Update media control playback state.
                            if let Some(media_controls) = &mut self.media_controls {
                                let progress = Some(souvlaki::MediaPosition(Duration::from_secs(
                                    time.seconds,
                                )));
                                if let Err(e) = media_controls
                                    .set_playback(souvlaki::MediaPlayback::Playing { progress })
                                {
                                    warn!("Audio - Couldn't update souvlaki playback: {e:#?}");
                                }
                            }
                        }
                    }
                    // We're done playing audio.
                    Err(symphonia::core::errors::Error::IoError(_err)) => {
                        self.skip(1, &mut AUDIO_STATE.write());
                        #[cfg(feature = "gui")]
                        gui_request_update();
                        continue;
                    }
                    Err(err) => warn!("audio error: {err}"),
                }
            }

            //------ End of `loop {}`.
        }
    }

    //-------------------------------------------------- Kernel message.
    #[inline(always)]
    fn kernel_msg(&mut self, msg: KernelToAudio) {
        use KernelToAudio::*;
        match msg {
            // Audio playback.
            Toggle => self.toggle(),
            Play => self.play(),
            Pause => self.pause(),
            Next => self.skip(1, &mut AUDIO_STATE.write()),
            Previous(threshold) => self.back(1, threshold, &mut AUDIO_STATE.write()),

            // Audio settings.
            Repeat(r) => self.repeat(r),
            Volume(v) => self.volume(v),

            // Queue.
            QueueAddSong((s_key, append, clear, play)) => {
                self.queue_add_song(s_key, append, clear, play)
            }
            QueueAddAlbum((al_key, append, clear, play, offset)) => {
                self.queue_add_album(al_key, append, clear, play, offset)
            }
            QueueAddArtist((ar_key, append, clear, play, offset)) => {
                self.queue_add_artist(ar_key, append, clear, play, offset)
            }
            QueueAddPlaylist((p, append, clear, play, offset)) => {
                self.queue_add_playlist(p, append, clear, play, offset)
            }
            Shuffle => self.shuffle(),
            Clear(play) => {
                self.clear(play, &mut AUDIO_STATE.write());
                #[cfg(feature = "gui")]
                gui_request_update();
            }
            Seek((seek, time)) => self.seek(seek, time, &mut AUDIO_STATE.write()),
            Skip(skip) => self.skip(skip, &mut AUDIO_STATE.write()),
            Back(back) => self.back(back, Some(0), &mut AUDIO_STATE.write()),

            // Queue Index.
            QueueSetIndex(idx) => self.queue_set_index(idx),
            QueueRemoveRange((range, next)) => self.queue_remove_range(range, next),

            // Audio State.
            RestoreAudioState => self.restore_audio_state(),
            SetOutputDevice(device) => self.set_audio_output_device(device),

            // Collection.
            DropCollection => self.drop_collection(),
            NewCollection(arc) => self.collection = arc,
        }
    }

    //-------------------------------------------------- Mc message.
    fn mc_msg(&mut self, event: souvlaki::MediaControlEvent) {
        use crate::audio::Seek;
        use souvlaki::{MediaControlEvent::*, SeekDirection};
        match event {
            Toggle => self.toggle(),
            Play => self.play(),
            Pause => self.pause(),
            Next => self.skip(1, &mut AUDIO_STATE.write()),
            Previous => self.back(1, None, &mut AUDIO_STATE.write()),
            Stop => {
                self.clear(false, &mut AUDIO_STATE.write());
                #[cfg(feature = "gui")]
                gui_request_update();
            }
            SetPosition(time) => {
                self.seek(Seek::Absolute, time.0.as_secs(), &mut AUDIO_STATE.write())
            }
            Seek(direction) => match direction {
                SeekDirection::Forward => self.seek(Seek::Forward, 5, &mut AUDIO_STATE.write()),
                SeekDirection::Backward => self.seek(Seek::Backward, 5, &mut AUDIO_STATE.write()),
            },
            SeekBy(direction, time) => match direction {
                SeekDirection::Forward => {
                    self.seek(Seek::Forward, time.as_secs(), &mut AUDIO_STATE.write())
                }
                SeekDirection::Backward => {
                    self.seek(Seek::Backward, time.as_secs(), &mut AUDIO_STATE.write())
                }
            },
            Raise => atomic_store!(MEDIA_CONTROLS_RAISE, true),
            Quit => atomic_store!(MEDIA_CONTROLS_SHOULD_EXIT, true),
            OpenUri(string) => warn!("Audio - Ignoring OpenURI({string})"),
        }
    }

    //-------------------------------------------------- Media control state setting.
    // Media Control state.
    fn set_media_controls_metadata(&mut self, key: SongKey) {
        if let Some(media_controls) = &mut self.media_controls {
            let (artist, album, song) = self.collection.walk(key);

            use disk::Plain;
            let mut _buf = String::new();
            let cover_url = match crate::collection::Image::base_path() {
                Ok(p) => {
                    // INVARIANT:
                    // `souvlaki` checks for `file://` prefix but
                    // the internal Windows impl _needs_ `\` as the
                    // separator or it will error.
                    _buf = format!(
                        "file://{}{}{}.jpg",
                        p.display(),
                        std::path::MAIN_SEPARATOR,
                        song.album
                    );
                    Some(_buf.as_str())
                }
                _ => None,
            };

            trace!(
				"Audio - set_media_controls_metadata({key}):\nsong.title: {},\nartist.name: {},\nalbum.title: {},\nsong.runtime: {},\ncover_url: {:?}",
				&song.title,
				&artist.name,
				&album.title,
				&song.runtime,
				&cover_url,
			);

            if let Err(e) = media_controls.set_metadata(souvlaki::MediaMetadata {
                title: Some(&song.title),
                artist: Some(&artist.name),
                album: Some(&album.title),
                duration: Some(Duration::from_secs(song.runtime.inner().into())),
                cover_url,
            }) {
                warn!("Audio - Couldn't update media controls metadata: {e:#?}");
            }
        } else {
            trace!("Audio - no media controls, ignoring set_media_controls_metadata({key})");
        }
    }

    // Media Control state.
    fn set_media_controls_progress(
        &mut self,
        state: &mut std::sync::RwLockWriteGuard<'_, AudioState>,
    ) {
        trace!(
            "Audio - set_media_controls_progress(), playing: {}, elapsed: {}",
            state.playing,
            state.elapsed
        );

        if let Some(media_controls) = &mut self.media_controls {
            let progress = Some(souvlaki::MediaPosition(Duration::from_secs(
                state.elapsed.inner().into(),
            )));
            let signal = match state.playing {
                true => souvlaki::MediaPlayback::Playing { progress },
                false => souvlaki::MediaPlayback::Paused { progress },
            };
            if let Err(e) = media_controls.set_playback(signal) {
                warn!("Audio - Couldn't update souvlaki playback: {e:#?}");
            }
        }
    }

    //-------------------------------------------------- Non-msg functions.
    // Error handling gets handled in these functions
    // rather than the caller (it gets called everywhere).

    // Convert a `SongKey` to a playable object.
    fn to_reader(&self, key: SongKey) -> Option<Box<dyn FormatReader>> {
        // Get `Song`.
        let song = &self.collection.songs[key];
        let path = &song.path;

        // Extension hint.
        let mut hint = Hint::new();
        hint.with_extension(&*song.extension);

        // Some misc `symphonia` options.
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts: MetadataOptions = Default::default();

        // Open file.
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                fail!("Audio - PATH error: {e} ... {}", path.display());
                send!(self.to_kernel, AudioToKernel::PathError((key, anyhow!(e))));
                return None;
            }
        };

        // Attempt probe.
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        match symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts) {
            Ok(o) => Some(o.format),
            Err(e) => {
                fail!("Audio - Probe error: {e} ... {}", path.display());
                send!(self.to_kernel, AudioToKernel::PathError((key, anyhow!(e))));
                None
            }
        }
    }

    // 1. Takes in a playable song from the above function
    // 2. Creates a decoder for the reader
    // 3. Sets our current state with it
    //
    // This does nothing on error.
    fn set_current(
        &mut self,
        reader: Box<dyn FormatReader>,
        timebase: TimeBase,
    ) -> Result<(), anyhow::Error> {
        // Select the first track with a known codec.
        let track = match reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        {
            Some(t) => t,
            None => return Err(anyhow!("Could not find track codec")),
        };

        // Create a decoder for the track.
        let decoder_opts = DecoderOptions { verify: false };
        let decoder =
            match symphonia::default::get_codecs().make(&track.codec_params, &decoder_opts) {
                Ok(d) => d,
                Err(e) => return Err(anyhow!(e)),
            };

        self.current = Some(AudioReader {
            reader,
            decoder,
            timebase,
            time: Time::new(0, 0.0),
        });

        Ok(())
    }

    // Convenience function that combines the above 2 functions.
    // Does nothing on error.
    fn set(&mut self, key: SongKey, state: &mut std::sync::RwLockWriteGuard<'_, AudioState>) {
        if let Some(reader) = self.to_reader(key) {
            // Discard any leftover audio samples.
            self.output.flush();

            if self
                .set_current(
                    reader,
                    TimeBase::new(1, self.collection.songs[key].sample_rate),
                )
                .is_ok()
            {
                // Set song state.
                state.song = Some(key);
                state.elapsed = Runtime::zero();
                state.runtime = self.collection.songs[key].runtime;
                #[cfg(feature = "gui")]
                gui_request_update();
                self.set_media_controls_metadata(key);
            }
        } else {
            self.clear(false, state);
        }
    }

    // Clears the `Queue`.
    //
    // The `bool` represents if we should
    // set `playing` to `false` in `AUDIO_STATE`.
    //
    // `false` is used when we want to clear
    // and then append new stuff and want to
    // prevent the API from flickering values.
    //
    // This takes in a lock so we can continue
    // more operations after this without flickering.
    fn clear(
        &mut self,
        keep_playing: bool,
        state: &mut std::sync::RwLockWriteGuard<'_, AudioState>,
    ) {
        trace!("Audio - clear({keep_playing})");

        state.queue.clear();
        state.playing = keep_playing;
        self.state.playing = keep_playing;

        if !keep_playing {
            state.finish();
            self.seek = None;
            self.current = None;
            if let Some(media_controls) = &mut self.media_controls {
                if let Err(e) = media_controls.set_playback(souvlaki::MediaPlayback::Stopped) {
                    warn!("Audio - Couldn't update souvlaki playback: {e:#?}");
                }
            }
        }
    }

    fn inner_play(&mut self, state: &mut std::sync::RwLockWriteGuard<'_, AudioState>) {
        let _ = self.output.play();
        self.state.playing = true;
        state.playing = true;
        self.set_media_controls_progress(state);
    }

    //-------------------------------------------------- Audio playback.
    fn toggle(&mut self) {
        if self.current.is_some() {
            flip!(self.state.playing);
            trace!("Audio - toggle(), playing: {}", self.state.playing);

            let mut state = AUDIO_STATE.write();
            flip!(state.playing);

            self.set_media_controls_progress(&mut state);

            #[cfg(feature = "gui")]
            gui_request_update();
        }
    }

    fn play(&mut self) {
        trace!("Audio - play()");
        if self.current.is_some() {
            self.inner_play(&mut AUDIO_STATE.write());

            #[cfg(feature = "gui")]
            gui_request_update();
        }
    }

    fn pause(&mut self) {
        trace!("Audio - pause()");
        if self.current.is_some() {
            let _ = self.output.pause();

            self.state.playing = false;

            let mut state = AUDIO_STATE.write();
            state.playing = false;
            self.set_media_controls_progress(&mut state);

            #[cfg(feature = "gui")]
            gui_request_update();
        }
    }

    fn skip(&mut self, skip: usize, state: &mut std::sync::RwLockWriteGuard<'_, AudioState>) {
        trace!("Audio - skip({skip})");

        if skip == 0 {
            return;
        }

        if state.repeat == Repeat::Song {
            if let Some(key) = state.song {
                trace!("Audio - repeating song: {key:?}");
                self.set(key, state);
            }
            return;
        }

        // For 1 skips.
        if skip == 1 {
            match state.next() {
                Some(next) => {
                    trace!("Audio - more songs left, setting: {next:?}");
                    self.set(next, state);
                }
                None => {
                    if matches!(state.repeat, Repeat::Queue | Repeat::QueuePause) {
                        if !state.queue.is_empty() {
                            let key = state.queue[0];
                            trace!("Audio - repeating queue, setting: {key:?}");
                            self.set(key, state);
                            state.song = Some(key);
                            state.queue_idx = Some(0);
                        }
                        if state.repeat == Repeat::QueuePause {
                            state.playing = false;
                            self.state.playing = false;
                        }
                    } else {
                        trace!("Audio - no songs left, calling state.finish()");
                        state.finish();
                        self.state.finish();
                        self.current = None;
                        #[cfg(feature = "gui")]
                        gui_request_update();
                    }
                }
            }
        // For >1 skips.
        } else if let Some(index) = state.queue_idx {
            let new_index = index + skip;
            let len = state.queue.len();

            // Repeat the queue if we're over bounds (and it's enabled).
            if matches!(state.repeat, Repeat::Queue | Repeat::QueuePause) && new_index >= len {
                if !state.queue.is_empty() {
                    let key = state.queue[0];
                    trace!("Audio - repeating queue, setting: {key:?}");
                    self.set(key, state);
                    state.song = Some(key);
                    state.queue_idx = Some(0);
                }
                if state.repeat == Repeat::QueuePause {
                    state.playing = false;
                    self.state.playing = false;
                }
            } else if len > new_index {
                let key = state.queue[new_index];
                self.set(key, state);
                state.song = Some(key);
                state.queue_idx = Some(new_index);
            } else {
                trace!("Audio - skip({new_index}) > {len}, calling state.finish()");
                state.finish();
                self.state.finish();
                self.current = None;
            }
        }

        #[cfg(feature = "gui")]
        gui_request_update();
    }

    fn back(
        &mut self,
        back: usize,
        threshold: Option<u32>,
        state: &mut std::sync::RwLockWriteGuard<'_, AudioState>,
    ) {
        let atomic_threshold = atomic_load!(PREVIOUS_THRESHOLD);
        trace!("Audio - back(back: {back}, threshold: {threshold:?}), atomic_threshold: {atomic_threshold}");

        if !state.queue.is_empty() {
            // FIXME:
            // Same as `skip()`.
            if let Some(index) = state.queue_idx {
                // Back input was greater than our current index,
                // play the first song in our queue.
                if index < back {
                    let key = state.queue[0];
                    self.set(key, state);
                    state.song = Some(key);
                    state.queue_idx = Some(0);
                } else if let Some(threshold) = threshold {
                    // Reset song if threshold.
                    if threshold != 0 && state.elapsed.inner() > threshold {
                        self.seek(Seek::Absolute, 0, state);
                    } else {
                        let new_index = index - back;
                        let key = state.queue[new_index];
                        self.set(key, state);
                        state.song = Some(key);
                        state.queue_idx = Some(new_index);
                    }
                } else if atomic_threshold != 0 && state.elapsed.inner() > atomic_threshold {
                    self.seek(Seek::Absolute, 0, state);
                } else {
                    let new_index = index - back;
                    let key = state.queue[new_index];
                    self.set(key, state);
                    state.song = Some(key);
                    state.queue_idx = Some(new_index);
                }

                #[cfg(feature = "gui")]
                gui_request_update();
            }
        }
    }

    fn seek(
        &mut self,
        seek: Seek,
        time: u64,
        state: &mut std::sync::RwLockWriteGuard<'_, AudioState>,
    ) {
        trace!("Audio - seek({seek:?}, {time})");

        if self.current.is_some() {
            let elapsed = state.elapsed.inner() as u64;
            let runtime = state.runtime.inner() as u64;

            match seek {
                Seek::Forward => {
                    let seconds = time + elapsed;

                    if seconds > runtime {
                        debug!("Audio - seek forward: {seconds} > {runtime}, calling .skip(1)");
                        self.skip(1, state);
                    } else {
                        self.seek = Some(symphonia::core::units::Time { seconds, frac: 0.0 });
                    }
                }
                Seek::Backward => {
                    let seconds = if time > elapsed {
                        debug!("Audio - seek backward: {time} > {elapsed}, setting to 0");
                        0
                    } else {
                        elapsed - time
                    };

                    self.seek = Some(symphonia::core::units::Time { seconds, frac: 0.0 });
                }
                Seek::Absolute => {
                    if time > runtime {
                        debug!("Audio - seek absolute: {time} > {runtime}, calling .skip(1)");
                        self.skip(1, state);
                    } else {
                        self.seek = Some(symphonia::core::units::Time {
                            seconds: time,
                            frac: 0.0,
                        });
                    }
                }
            }
        }

        #[cfg(feature = "gui")]
        gui_request_update();
    }

    //-------------------------------------------------- Audio settings.
    fn shuffle(&mut self) {
        trace!("Audio - Shuffle");

        let mut state = AUDIO_STATE.write();

        if !state.queue.is_empty() {
            use rand::{prelude::SliceRandom, SeedableRng};
            let mut rng = rand::rngs::SmallRng::from_entropy();

            state.queue.make_contiguous().shuffle(&mut rng);
            state.queue_idx = Some(0);
            self.set(state.queue[0], &mut state);
        }
    }

    fn repeat(&mut self, repeat: Repeat) {
        trace!("Audio - Repeat::{repeat:?}");
        AUDIO_STATE.write().repeat = repeat;
    }

    fn volume(&mut self, volume: Volume) {
        trace!("Audio - {volume:?}");
        atomic_store!(VOLUME, volume.inner());

        AUDIO_STATE.write().volume = volume;

        #[cfg(feature = "gui")]
        gui_request_update();
    }

    //-------------------------------------------------- Queue.
    fn queue_add_song(&mut self, key: SongKey, append: Append, clear: bool, play: bool) {
        trace!("Audio - queue_add_song({key:?}, {append:?}, {clear}, {play})");

        let mut state = AUDIO_STATE.write();

        if clear {
            self.clear(play, &mut state)
        }

        match append {
            Append::Back => {
                state.queue.push_back(key);
                if self.current.is_none() {
                    state.queue_idx = Some(0);
                    self.set(key, &mut state);
                }
            }
            Append::Front => {
                state.queue.push_front(key);
                state.queue_idx = Some(0);
                self.set(key, &mut state);
            }
            Append::Index(i) => {
                if i == 0 {
                    state.queue_idx = Some(0);
                    self.set(key, &mut state);
                }
                state.queue.insert(i, key);
            }
        }

        if !clear && play {
            self.inner_play(&mut state);
        }
    }

    fn queue_add_album(
        &mut self,
        key: AlbumKey,
        append: Append,
        clear: bool,
        play: bool,
        offset: usize,
    ) {
        trace!("Audio - queue_add_album({key:?}, {append:?}, {clear}, {play}, {offset})");

        let mut state = AUDIO_STATE.write();

        if clear {
            self.clear(play, &mut state)
        }

        // Prevent bad offsets panicking.
        let offset = if offset >= self.collection.albums[key].songs.len() {
            0
        } else {
            offset
        };

        let album = &self.collection.albums[key];

        // INVARIANT:
        // `Collection` only creates `Album`'s that
        // have a minimum of 1 `Song`, so this should
        // never panic.
        let keys = album.songs.iter();
        match append {
            Append::Back => {
                keys.for_each(|k| state.queue.push_back(*k));
                if self.current.is_none() {
                    state.queue_idx = Some(offset);
                    self.set(self.collection.albums[key].songs[offset], &mut state);
                }
            }
            Append::Front => {
                keys.rev().for_each(|k| state.queue.push_front(*k));
                state.queue_idx = Some(offset);
                self.set(self.collection.albums[key].songs[offset], &mut state);
            }
            Append::Index(mut i) => {
                let og_i = i;
                keys.for_each(|k| {
                    state.queue.insert(i, *k);
                    i += 1;
                });
                if og_i == 0 {
                    state.queue_idx = Some(0);
                    self.set(self.collection.albums[key].songs[offset], &mut state);
                }
            }
        }

        if !clear && play {
            self.inner_play(&mut state);
        }
    }

    fn queue_add_artist(
        &mut self,
        key: ArtistKey,
        append: Append,
        clear: bool,
        play: bool,
        offset: usize,
    ) {
        trace!("Audio - queue_add_artist({key:?}, {append:?}, {clear}, {play}, {offset}");

        let keys: Box<[SongKey]> = self.collection.all_songs(key);

        let mut state = AUDIO_STATE.write();

        if clear {
            self.clear(play, &mut state)
        }

        // Prevent bad offsets panicking.
        let offset = if offset >= keys.len() { 0 } else { offset };

        // INVARIANT:
        // `Collection` only creates `Artist`'s that
        // have a minimum of 1 `Song`, so this should
        // never panic.
        let iter = keys.iter();
        match append {
            Append::Back => {
                iter.for_each(|k| state.queue.push_back(*k));
                if self.current.is_none() {
                    state.queue_idx = Some(offset);
                    self.set(keys[offset], &mut state);
                }
            }
            Append::Front => {
                iter.rev().for_each(|k| state.queue.push_front(*k));
                state.queue_idx = Some(offset);
                self.set(keys[offset], &mut state);
            }
            Append::Index(mut i) => {
                if i == 0 {
                    state.queue_idx = Some(0);
                    self.set(keys[offset], &mut state);
                }
                iter.for_each(|k| {
                    state.queue.insert(i, *k);
                    i += 1;
                });
            }
        }

        if !clear && play {
            self.inner_play(&mut state);
        }
    }

    fn queue_add_playlist(
        &mut self,
        playlist: Arc<str>,
        append: Append,
        clear: bool,
        play: bool,
        offset: usize,
    ) {
        trace!("Audio - queue_add_playlist({playlist}, {append:?}, {clear}, {play}, {offset}");

        let Some(keys) = crate::state::PLAYLISTS
            .read()
            .valid_keys(&playlist, &self.collection)
        else {
            trace!("Audio - {playlist} doesn't exist, skipping");
            return;
        };

        if keys.is_empty() {
            trace!("Audio - {playlist} is empty, skipping");
            return;
        };

        let mut state = AUDIO_STATE.write();

        if clear {
            self.clear(play, &mut state)
        }

        // Prevent bad offsets panicking.
        let offset = if offset >= keys.len() { 0 } else { offset };

        // INVARIANT:
        // `Collection` only creates `Artist`'s that
        // have a minimum of 1 `Song`, so this should
        // never panic.
        let iter = keys.iter();
        match append {
            Append::Back => {
                iter.for_each(|k| state.queue.push_back(*k));
                if self.current.is_none() {
                    state.queue_idx = Some(offset);
                    self.set(keys[offset], &mut state);
                }
            }
            Append::Front => {
                iter.rev().for_each(|k| state.queue.push_front(*k));
                state.queue_idx = Some(offset);
                self.set(keys[offset], &mut state);
            }
            Append::Index(mut i) => {
                iter.for_each(|k| {
                    state.queue.insert(i, *k);
                    i += 1;
                });
                if i == 0 {
                    state.queue_idx = Some(0);
                    self.set(keys[offset], &mut state);
                }
            }
        }

        if !clear && play {
            self.inner_play(&mut state);
        }
    }

    fn queue_set_index(&mut self, index: usize) {
        let mut state = AUDIO_STATE.write();

        // Prevent bad index panicking.
        let len = state.queue.len();
        if index >= state.queue.len() {
            trace!("Audio - queue_set_index({index}) >= {len}, calling .finish()");
            self.state.finish();
            state.finish();
            self.current = None;
        } else {
            trace!("Audio - queue_set_index({index})");
            state.queue_idx = Some(index);
            self.set(state.queue[index], &mut state);
        }

        #[cfg(feature = "gui")]
        gui_request_update();
    }

    fn queue_remove_range(&mut self, range: std::ops::Range<usize>, next: bool) {
        let start = range.start;
        let end = range.end;

        #[cfg(debug_assertions)]
        if start >= end {
            debug_panic!("Audio - queue_remove_range({start} >= {end}");
        }

        let mut state = AUDIO_STATE.write();

        let len = state.queue.len();
        let contains = if let Some(i) = state.queue_idx {
            range.contains(&i)
        } else {
            false
        };

        // Prevent bad start/end panicking.
        if start >= len {
            warn!("Audio - start is invalid, skipping queue_remove_range({range:?})");
            return;
        } else if end > len {
            warn!("Audio - end is invalid, skipping queue_remove_range({range:?})");
            return;
        }

        trace!("Audio - queue_remove_range({range:?})");
        state.queue.drain(range);

        // Figure out the real `queue_idx` position after draining.
        if let Some(index) = state.queue_idx {
            // If the start is 0 and our index got wiped, we should reset to 0.
            if start == 0 && contains && len > end {
                let _new = 0;
                state.queue_idx = Some(0);
                trace!("Audio - queue_remove_range({start}..{end}), beginning index: 0");
                if next {
                    self.set(state.queue[0], &mut state);
                }
            // If we deleted our current index, but there's
            // more songs ahead of us, don't change the current index,
            // just set the new song that index represents.
            } else if next && index == start {
                if let Some(key) = state.queue.get(index) {
                    self.set(*key, &mut state);
                    trace!("Audio - queue_remove_range({start}..{end}), resetting current index: {index}");
                }
            // If the current index is greater than the end, e.g:
            //
            // [0]
            // [1] <- start
            // [2]
            // [3] <- end
            // [4]
            // [5] <- current queue_idx
            // [6]
            //
            // We should subtract the queue_idx so it lines up
            // correctly. In the above case we are taking out 3 elements,
            // so the current queue_idx should go from 5 to (5 - 3), so element 2:
            //
            // [0]
            // [1] (used to be [4])
            // [2] <- new queue_idx
            // [3] (used to be [6])
            //
            } else if index >= end {
                let new = end - start;
                state.queue_idx = Some(index - new);
                trace!("Audio - queue_remove_range({start}..{end}), new index: {new}");
            }
        }

        #[cfg(feature = "gui")]
        gui_request_update();
    }

    //-------------------------------------------------- Restore Audio State.
    // Sets the global `AUDIO_STATE` to our local `self.state`.
    fn restore_audio_state(&mut self) {
        trace!("Audio - restore_audio_state()");

        let mut state = AUDIO_STATE.write();

        // INVARIANT:
        // `Kernel` validates `AUDIO_STATE` before handing
        // it off to `Audio` so we should be safe to assume
        // the state holds proper indices into the `Collection`
        // and into itself.
        trace!("Audio - Restoring: {:#?}", self.state);
        *state = self.state.clone();

        atomic_store!(VOLUME, state.volume.inner());

        if let Some(key) = self.state.song {
            // Start playback.
            let elapsed = state.elapsed.inner() as u64;
            debug!("Audio - Restore ... setting {key:?}");
            self.set(key, &mut state);

            // HACK:
            // The above `set()` resets some of the state, so re-copy.
            self.state.if_copy(&mut state);

            // Restore media control progress.
            // (the metadata is set above in `.set()`)
            //
            // HACK:
            // If playback is paused, the media controls won't
            // register until a sample is played. To work around
            // this, set the media controls to `playing` briefly
            // if we're paused.
            if !state.playing {
                state.playing = true;
                self.set_media_controls_progress(&mut state);
                state.playing = false;
            }
            self.set_media_controls_progress(&mut state);

            if elapsed > 0 {
                self.seek(Seek::Absolute, elapsed, &mut state);
                debug!(
                    "Audio - Restore ... seeking {}/{}",
                    state.elapsed, state.runtime
                );
            } else {
                debug!("Audio - Restore ... skipping seek");
            }

            #[cfg(feature = "gui")]
            gui_request_update();
        }
    }

    fn set_audio_output_device(&mut self, device: AudioOutputDevice) {
        info!("Audio - set_audio_output_device({device:?}");
        match self.output.change_output_device(device) {
            Ok(_) => {
                info!("Audio - Successfully changed audio output device");
            }
            Err(e) => {
                error!("Audio - Couldn't change audio output device: {e:#?}");
            }
        }

    }

    //-------------------------------------------------- Collection.
    fn drop_collection(&mut self) {
        // Drop pointer.
        self.collection = Collection::dummy();

        // Hang until we get the new one.
        debug!("Audio - Dropped Collection, waiting...");

        // Ignore messages until it's a pointer.
        loop {
            match recv!(self.from_kernel) {
                KernelToAudio::NewCollection(arc) => {
                    ok_debug!("Audio - New Collection received");
                    self.collection = arc;

                    // INVARIANT:
                    // `AUDIO_STATE` _should_ be set valid by `Kernel` at this point.
                    self.state = AUDIO_STATE.read().clone();
                    if self.state.song.is_none() {
                        self.current = None;
                    }

                    return;
                }
                _ => {
                    debug_panic!("Audio - Incorrect message received");
                    error!("Audio - Incorrect message received");
                }
            }
        }
    }
}

//---------------------------------------------------------------------------------------------------- TESTS
//#[cfg(test)]
//mod tests {
//  #[test]
//  fn __TEST__() {
//  }
//}
