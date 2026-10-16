use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Default)]
pub(crate) struct Channel {
    pub queue: VecDeque<i16>,
    pub enabled: bool,
    pub primed: bool,
    pub underruns: u64,
    pub prefill: usize,
}
pub(crate) type Samples = Arc<Mutex<Channel>>;
type Mixer = Arc<Mutex<BTreeMap<u64, Samples>>>;

pub(crate) struct Sound {
    pub rate: u32,
    pub enabled: Arc<AtomicBool>,
    channels: Mixer,
    pub error: Arc<Mutex<Option<String>>>,
    _stream: Option<cpal::Stream>,
}

impl Sound {
    pub fn new() -> Self {
        let channels = Mixer::default();
        let error = Arc::default();
        let enabled = Arc::new(AtomicBool::new(true));
        match Self::stream(channels.clone(), Arc::clone(&error), enabled.clone()) {
            Ok((stream, rate)) => Self {
                rate,
                enabled,
                channels,
                error,
                _stream: Some(stream),
            },
            Err(failure) => {
                *error.lock().unwrap() = Some(format!("Audio unavailable: {failure:#}"));
                Self {
                    rate: 48_000,
                    enabled,
                    channels,
                    error,
                    _stream: None,
                }
            }
        }
    }

    fn stream(
        channels: Mixer,
        error: Arc<Mutex<Option<String>>>,
        enabled: Arc<AtomicBool>,
    ) -> Result<(cpal::Stream, u32)> {
        let device = cpal::default_host()
            .default_output_device()
            .context("No output device")?;
        let supported = device.default_output_config()?;
        let config: cpal::StreamConfig = supported.clone().into();
        let count = config.channels as usize;
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => {
                build::<f32>(&device, &config, count, channels, error, enabled)?
            }
            cpal::SampleFormat::I16 => {
                build::<i16>(&device, &config, count, channels, error, enabled)?
            }
            cpal::SampleFormat::U16 => {
                build::<u16>(&device, &config, count, channels, error, enabled)?
            }
            format => anyhow::bail!("Unsupported audio format {format:?}"),
        };
        stream.play()?;
        Ok((stream, config.sample_rate))
    }

    pub fn add(&mut self, id: u64) -> Samples {
        let queue = Arc::new(Mutex::new(Channel {
            prefill: self.rate as usize / 25,
            ..Channel::default()
        }));
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
    enabled: Arc<AtomicBool>,
) -> Result<cpal::Stream> {
    let prefill = config.sample_rate as usize / 25;
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            let enabled = enabled.load(Ordering::Relaxed);
            let channels = channels.lock().unwrap();
            let mut sources: Vec<_> = channels.values().map(|q| q.lock().unwrap()).collect();
            for source in &mut sources {
                source.prefill = prefill.max(output.len() / count * 2);
                if source.enabled && source.queue.len() >= source.prefill {
                    source.primed = true;
                }
            }
            for frame in output.chunks_mut(count) {
                let sum: f32 = sources
                    .iter_mut()
                    .filter_map(|source| {
                        if !source.enabled || !source.primed {
                            return None;
                        }
                        let sample = source.queue.pop_front();
                        if sample.is_none() {
                            source.primed = false;
                            source.underruns += 1;
                        }
                        sample
                    })
                    .map(|s| f32::from(s) / 32768.)
                    .sum();
                frame.fill(T::from_sample(if enabled {
                    sum.clamp(-1., 1.)
                } else {
                    0.
                }));
            }
        },
        move |failure| {
            *error.lock().unwrap() = Some(format!("Audio unavailable: {failure}"));
        },
        None,
    )?)
}
