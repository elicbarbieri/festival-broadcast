use anyhow::anyhow;
use cpal::Device;
use cpal::traits::{DeviceTrait, HostTrait};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use crate::audio::output::AudioOutputError;

use once_cell::sync::Lazy;
use std::sync::RwLock;
use egui::TextBuffer;
use strum::Display;

// Lazy initialization of audio devices with a RwLock for read/write access
// UI reads this every frame if selecting device in settings
static AUDIO_OUTPUT_DEVICES: Lazy<RwLock<Vec<String>>> = Lazy::new(|| {
    let devices = AudioOutputDevice::read_available_devices();
    RwLock::new(devices)
});



/// Audio output device.
#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
pub struct AudioOutputDevice {
    /// Name of the audio output device.
    pub device_name: String
}

impl AudioOutputDevice {

    /// Converts the `AudioOutputDevice` into a `cpal::Device`.  If
    /// the device name is not found, or multiple devices are found,
    /// an error is returned.
    pub fn get_cpal_device(&self) -> Result<Device, AudioOutputError> {
        let host = cpal::default_host();
        if self.device_name == "Default Device" {
            return host.default_output_device().ok_or(
                AudioOutputError::InvalidOutputDevice(anyhow!("No default output device found"))
            );
        }

        let devices = host.output_devices().map_err(|e| AudioOutputError::InvalidOutputDevice(anyhow!(e)))?;

        let mut matching_devices = devices.into_iter()
            .filter(|d| d.name().map(|n| n == self.device_name).unwrap_or(false))
            .collect::<Vec<Device>>();

        if matching_devices.is_empty() {
            return Err(AudioOutputError::InvalidOutputDevice(
                anyhow!("No matching device found with name {}", self.device_name))
            );
        }

        if matching_devices.len() == 1 {
            Ok(matching_devices.remove(0))
        } else {
            Err(AudioOutputError::InvalidOutputDevice(
                anyhow!("Multiple devices found with name {}", self.device_name))
            )
        }
    }

    /// Converts a string into an `AudioOutputDevice`.
    pub fn from_str(device_name: &str) -> Self {
        Self {
            device_name: device_name.to_string()
        }
    }

    /// Human readable name for audio device
    pub fn human(&self) -> String {
        self.device_name.clone()
    }

    /// Returns a list of available audio output devices.  The returned String can be mapped
    /// back to a cpal `Device` using `get_cpal_device`.
    pub fn available_devices() -> Vec<String> {
        let devices = AUDIO_OUTPUT_DEVICES.read().unwrap();
        devices.clone()
    }

    /// Reads the available audio output devices from the system
    fn read_available_devices() -> Vec<String> {
        let devices = match cpal::default_host().output_devices() {
            Ok(devices) => devices,
            Err(err) => {
                error!("Audio - failed to get output devices: {err}");
                return Vec::new();
            }
        };

        let mut valid_devices: Vec<String> = devices
            .filter(|d| d.name().is_ok())
            .map(|d| d.name().unwrap())
            .collect();

        valid_devices.insert(0, "Default Device".to_string());
        valid_devices
    }

    /// Updates the list of available audio output devices.
    pub fn update_available_devices() {
        let mut devices = AUDIO_OUTPUT_DEVICES.write().unwrap();
        *devices = AudioOutputDevice::read_available_devices();
    }

    /// Returns true if the device can be connected to.
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
}

impl Iterator for AudioOutputDevice {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        let devices = AUDIO_OUTPUT_DEVICES.read().unwrap();
        devices.iter().next().cloned()
    }
}

impl Default for AudioOutputDevice {
    fn default() -> Self {
        AudioOutputDevice {
            device_name: "Default Device".to_string()
        }
    }
}