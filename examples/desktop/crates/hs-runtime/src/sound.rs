use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

pub(crate) type Samples = Arc<Mutex<VecDeque<i16>>>;
type Mixer = Arc<Mutex<BTreeMap<u64, Samples>>>;

pub(crate) struct Sound {
    pub rate: u32,
    channels: Mixer,
    pub error: Arc<Mutex<Option<String>>>,
    _stream: Option<cpal::Stream>,
}

impl Sound {
    pub fn new() -> Self {
        let channels = Mixer::default();
        let error = Arc::default();
        match Self::stream(channels.clone(), Arc::clone(&error)) {
            Ok((stream, rate)) => Self {
                rate,
                channels,
                error,
                _stream: Some(stream),
            },
            Err(failure) => {
                *error.lock().unwrap() = Some(format!("Audio unavailable: {failure:#}"));
                Self {
                    rate: 48_000,
                    channels,
                    error,
                    _stream: None,
                }
            }
        }
    }

    fn stream(channels: Mixer, error: Arc<Mutex<Option<String>>>) -> Result<(cpal::Stream, u32)> {
        let device = cpal::default_host()
            .default_output_device()
            .context("No output device")?;
        let supported = device.default_output_config()?;
        let config: cpal::StreamConfig = supported.clone().into();
        let count = config.channels as usize;
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, count, channels, error)?,
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, count, channels, error)?,
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, count, channels, error)?,
            format => anyhow::bail!("Unsupported audio format {format:?}"),
        };
        stream.play()?;
        Ok((stream, config.sample_rate))
    }

    pub fn add(&mut self, id: u64) -> Samples {
        let queue = Samples::default();
        self.channels.lock().unwrap().insert(id, queue.clone());
        queue
    }
    pub fn remove(&mut self, id: u64) {
        self.channels.lock().unwrap().remove(&id);
    }
}

fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    count: usize,
    channels: Mixer,
    error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream> {
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            let channels = channels.lock().unwrap();
            let mut sources: Vec<_> = channels.values().map(|q| q.lock().unwrap()).collect();
            let audible = sources.iter().filter(|q| !q.is_empty()).count().max(1) as f32;
            for frame in output.chunks_mut(count) {
                let sum: f32 = sources
                    .iter_mut()
                    .filter_map(|q| q.pop_front())
                    .map(|s| f32::from(s) / 32768.)
                    .sum();
                frame.fill(T::from_sample(sum / audible));
            }
        },
        move |failure| {
            *error.lock().unwrap() = Some(format!("Audio unavailable: {failure}"));
        },
        None,
    )?)
}
