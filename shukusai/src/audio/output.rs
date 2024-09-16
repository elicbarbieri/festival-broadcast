// Symphonia
// Copyright (c) 2019-2022 The Project Symphonia Developers.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// This code is a modified version of:
// `https://github.com/pdeljanov/Symphonia/blob/master/symphonia-play/src/output.rs`

//---------------------------------------------------------------------------------------------------- Use
use crate::audio::Volume;
use crate::constants::FESTIVAL;
use crate::state::VOLUME;
use anyhow::anyhow;
use benri::{atomic_load, sleep};
use symphonia::core::audio::*;
// use crate::audio::resampler::Resampler;

use symphonia::core::audio::{AudioBufferRef, RawSample, SampleBuffer, SignalSpec};
use symphonia::core::units::Duration;

use cpal::{Device, StreamConfig};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rb::{Producer, Consumer, SpscRb, RB, RbInspector, RbProducer, RbConsumer};
use log::{debug, error, info, trace, warn};

// ----------------------------------------------------------------------------------- Constants
// If the audio device is not connected, how many seconds
// should we wait before trying to connect again?
const RETRY_SECONDS: u64 = 1;


#[derive(Debug)]
pub(crate) enum AudioOutputError {
    OpenStream(anyhow::Error),
    PlayStream(anyhow::Error),
    StreamClosed(anyhow::Error),
    InvalidOutputDevice(anyhow::Error),
    Channel(anyhow::Error),
    InvalidSpec(anyhow::Error),
    NonF32(anyhow::Error),
    Resampler(anyhow::Error),
}

impl AudioOutputError {
    pub(crate) fn into_anyhow(self) -> anyhow::Error {
        use AudioOutputError::*;
        match self {
            OpenStream(a) => a,
            PlayStream(a) => a,
            StreamClosed(a) => a,
            InvalidOutputDevice(a) => a,
            Channel(a) => a,
            InvalidSpec(a) => a,
            NonF32(a) => a,
            Resampler(a) => a,
        }
    }
}

pub(crate) struct AudioOutputDevice{  // TODO:  Make things private that can be
    pub device_name: String,
    pub is_default: bool,
    pub volume: Volume
}

impl AudioOutputDevice {
    pub fn into_cpal_device(self) -> Result<Device, AudioOutputError> {
        let host = cpal::default_host();
        let devices = host.output_devices().map_err(|e| AudioOutputError::InvalidOutputDevice(anyhow!(e)))?;

        let mut matching_devices = devices.into_iter()
            .filter(|d| d.name().map(|n| n == self.device_name).unwrap_or(false))
            .collect::<Vec<Device>>();

        if matching_devices.is_empty() {
            return Err(AudioOutputError::InvalidOutputDevice(
                anyhow!("No matching device found with name {}", self.device_name))
            );
        }

        if matching_devices.len() == 0 {
            Ok(matching_devices.remove(0))
        } else {
            Err(AudioOutputError::InvalidOutputDevice(
                anyhow!("Multiple devices found with name {}", self.device_name))
            )
        }
    }
}


    // SOMEDAY: support i16/u16.
pub(crate) struct AudioOutput {
    ring_buf: SpscRb<f32>,
    ring_buf_producer: Producer<f32>,
    sample_buf: SampleBuffer<f32>,
    stream: cpal::Stream,
    // resampler: Option<Resampler<f32>>,
    samples: Vec<f32>,

    pub(crate) device: Device,
    pub(crate) spec: SignalSpec,
    pub(crate) duration: Duration,
}

impl AudioOutput {

    pub fn can_connect(device: &Device) -> bool {
        let config = match device.default_output_config() {
            Ok(config) => config,
            Err(_) => return false,
        };

        let stream = device.build_output_stream(
            &config.config(),
            |data: &mut [f32], _: &cpal::OutputCallbackInfo| {},
            |err| warn!("Audio - audio output error: {err}"),
            Some(core::time::Duration::from_millis(100)),
        );
        let connectable = stream.is_ok();
        drop(stream);

        connectable
    }

    pub fn available_devices() -> Vec<Device> {
        let devices = match cpal::default_host().output_devices() {
            Ok(devices) => devices,
            Err(err) => {
                error!("Audio - failed to get output devices: {err}");
                return Vec::new();
            }
        };

        devices.collect()
    }

    fn build_config(device: &Device, signal_spec: &SignalSpec) -> Result<StreamConfig, AudioOutputError> {

        let config = match device.default_output_config() {
            Ok(config) => config,
            Err(err) => return Err(AudioOutputError::OpenStream(anyhow!(err))),
        };

        // SOMEDAY: support i16/u16.
        if config.sample_format() != cpal::SampleFormat::F32 {
            return Err(AudioOutputError::NonF32(anyhow!(
                "sample format is not f32"
            )));
        }

        let num_channels = signal_spec.channels.count();

        // Output audio stream config.
        #[cfg(windows)]
        let config = config.config();
        #[cfg(unix)]
        let config = cpal::StreamConfig {
            channels: num_channels as cpal::ChannelCount,
            sample_rate: cpal::SampleRate(signal_spec.rate),
            buffer_size: cpal::BufferSize::Default,
        };

        Ok(config)
    }

    pub fn try_open(
        device: Option<Device>,
        spec: Option<SignalSpec>,
        duration: Duration,
    ) -> Result<Self, AudioOutputError> {
        let device = match device {
            Some(device) => device,
            None => {
                match cpal::default_host().default_output_device() {
                    Some(device) => device,
                    None => return Err(
                        AudioOutputError::OpenStream(anyhow!("no default audio output device"))
                    )
                }
            }
        };

        let device_name = device.name().unwrap_or_else(|_| "Un-Named Device".to_string());

        let spec = spec.unwrap_or_else(|| SignalSpec {
            rate: 44_100,
            channels: Channels::FRONT_LEFT,
        });

        let num_channels = spec.channels.count();

        // Create a ring buffer with a capacity for up-to 50ms of audio.
        let ring_len = ((50 * spec.rate as usize) / 1000) * num_channels;
        let ring_buf = SpscRb::new(ring_len);
        let ring_buf_producer = ring_buf.producer();

        let config = Self::build_config(&device, &spec)?;

        let mut tries = 0_usize;

        let stream = loop {
            let ring_buf_consumer = ring_buf.consumer();

            let stream_result = device.build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // Write out as many samples as possible from the ring buffer to the audio output.
                    let written = ring_buf_consumer.read(data).unwrap_or(0);

                    // Mute any remaining samples.
                    data[written..].fill(0.0);
                },
                move |err| warn!("Audio - audio output error: {err}"),
                None,  // TODO: find sane value for stream timeout
            );

            match stream_result {
                Ok(s) => {
                    debug!("Audio Init [1/3] ... connected to {} @ {}Hz", device_name, spec.rate);
                    break s;
                }
                Err(e) => {
                    if tries == 5 {
                        warn!("Audio Init [1/3] ... output device {device_name} error: {e:?} ... will continue to retry every {RETRY_SECONDS} seconds, but will only log when we succeed");
                    } else if tries < 5 {
                        warn!("Audio Init [1/3] ... output device {device_name} error: {e:?} ... retrying in {RETRY_SECONDS} seconds");
                    }
                    tries += 1;
                }
            }
            sleep!(RETRY_SECONDS);
        };

        // Start the output stream.
        if let Err(err) = stream.play() {
            return Err(AudioOutputError::PlayStream(anyhow!(err)));
        }

        let sample_buf = SampleBuffer::<f32>::new(duration, spec);

        // FIXME: Handle resampling

        // To make testing easier, always enable the
        // resampler if this env variable is specified.
        //
        // Else, fallback to if we actually need it or not.
        // let resampler_needed = if std::env::var_os("FESTIVAL_FORCE_RESAMPLE").is_some() {
        //     info!("FESTIVAL_FORCE_RESAMPLE detected, creating resampler");
        //     true
        // } else {
        //     spec.rate != config.sample_rate.0
        // };

        // let resampler = if resampler_needed {
        //     debug!("Audio - resampling {spec.rate}Hz to {config.sample_rate.0}Hz");
        //
        //     match Resampler::new(spec, config.sample_rate.0 as usize, duration) {
        //         Ok(r) => Some(r),
        //         Err(e) => {
        //             error!("Audio - failed to create resampler: {e}");
        //             return Err(AudioOutputError::Resampler(anyhow!(e)));
        //         }
        //     }
        // } else {
        //     debug!("Audio - no resampling needed for {}Hz", spec.rate);
        //     None
        // };

        let samples = Vec::with_capacity(num_channels * duration as usize);

        Ok(Self {
            device,
            ring_buf,
            ring_buf_producer,
            sample_buf,
            samples,
            stream,
            // resampler,
            spec,
            duration,
        })
    }

    pub fn reopen_stream(&mut self, signal_spec: &SignalSpec, duration: &Duration) -> Result<(), AudioOutputError> {
        let config = Self::build_config(&self.device, signal_spec).unwrap();

        let num_channels = signal_spec.channels.count();
        let ring_len = ((50 * signal_spec.rate as usize) / 1000) * num_channels;
        let ring_buf = SpscRb::new(ring_len);
        let (ring_buf_producer, ring_buf_consumer) = (ring_buf.producer(), ring_buf.consumer());

        let stream_result = self.device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Write out as many samples as possible from the ring buffer to the audio output.
                let written = ring_buf_consumer.read(data).unwrap_or(0);

                // Mute any remaining samples.
                data[written..].fill(0.0);
            },
            move |err| warn!("Audio - audio output error: {err}"),
            None,  // TODO: find sane value for stream timeout
        );

        let stream = match stream_result {
            Ok(s) => s,
            Err(e) => return Err(AudioOutputError::OpenStream(anyhow!(e))),
        };

        if let Err(err) = stream.play() {
            return Err(AudioOutputError::PlayStream(anyhow!(err)))
        }

        self.ring_buf = ring_buf;
        self.ring_buf_producer = ring_buf_producer;
        self.stream = stream;
        self.sample_buf = SampleBuffer::<f32>::new(*duration, *signal_spec);
        self.samples = Vec::with_capacity(num_channels * *duration as usize);

        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), AudioOutputError> {
        self.flush();
        self.stream
            .pause()
            .map_err(|e| AudioOutputError::PlayStream(anyhow!("pause error")))
    }

    pub fn play(&mut self) -> Result<(), AudioOutputError> {
        self.stream
            .play()
            .map_err(|e| AudioOutputError::PlayStream(anyhow!("play error")))
    }

    pub fn write(
        &mut self,
        decoded: AudioBufferRef<'_>,
    ) -> Result<(), AudioOutputError> {
        // Do nothing if there are no audio frames.
        if decoded.frames() == 0 {
            return Ok(());
        }

        let capacity = decoded.capacity();

        // let samples = if let Some(resampler) = &mut self.resampler {
        //     // Resampling is required. The resampler will return interleaved samples in the
        //     // correct sample format.
        //     match resampler.resample(decoded) {
        //         Ok(resampled) => resampled,
        //         Err(e) => {
        //             trace!("Audio - write(): {e}");
        //             return Err(AudioOutputError::Resampler(e));
        //         }
        //     }
        // } else {
        //     // Resampling is not required. Interleave the sample for cpal using a sample buffer.
        //
        // };

        self.sample_buf.copy_interleaved_ref(decoded);
        let samples = self.sample_buf.samples();

        self.samples.clear();
        self.samples.extend_from_slice(samples);

        // Apply volume transformation.
        let volume = Volume::new(atomic_load!(VOLUME)).f32();

        // Taken from: https://docs.rs/symphonia-core/0.5.3/src/symphonia_core/audio.rs.html#680-692
        //
        // Changed to use iterators over indexing.
        self.samples
            .chunks_mut(capacity)
            .for_each(|plane| plane.iter_mut().for_each(|sample| *sample *= volume));

        let mut samples = self.samples.as_slice();

        // Write all samples to the ring buffer.
        while let Some(written) = self.ring_buf_producer.write_blocking(samples) {
            samples = &samples[written..];
        }

        Ok(())
    }

    pub fn flush(&mut self) {
        // INVARIANT:
        // The resampled samples all get written immediately
        // after production, so there are no "old" samples
        // left in `self.resampler`, all of them are in
        // the ring_buffer, so just wait until it is empty.
        // Maximum wait time is 50ms.
        while !self.ring_buf.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
