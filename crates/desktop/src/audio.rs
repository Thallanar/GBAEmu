//! Saída de áudio no host via `cpal`.
//!
//! O APU do core gera amostras i16 intercaladas (L,R) a [`auroragba_core::apu::
//! OUTPUT_RATE`] (32768 Hz). Aqui reamostramos para a taxa do dispositivo e
//! empurramos num ring buffer compartilhado; o callback do `cpal` (em outra
//! thread) drena esse ring. Em underrun, sai silêncio.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct AudioOut {
    // O stream precisa ficar vivo enquanto tocamos.
    _stream: cpal::Stream,
    ring: Arc<Mutex<VecDeque<f32>>>,
    sample_rate: u32,
    channels: u16,
    /// Posição fracionária (em frames de entrada) do reamostrador.
    phase: f64,
}

impl AudioOut {
    pub fn new() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let config = device.default_output_config().ok()?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        log::info!("Áudio: {sample_rate} Hz, {channels} canais");

        let ring: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let ring_cb = ring.clone();
        let err_fn = |e| log::error!("erro de áudio: {e}");
        let sc: cpal::StreamConfig = config.clone().into();

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &sc,
                move |data: &mut [f32], _| {
                    let mut r = ring_cb.lock().unwrap();
                    for s in data.iter_mut() {
                        *s = r.pop_front().unwrap_or(0.0);
                    }
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &sc,
                move |data: &mut [i16], _| {
                    let mut r = ring_cb.lock().unwrap();
                    for s in data.iter_mut() {
                        *s = (r.pop_front().unwrap_or(0.0) * 32767.0) as i16;
                    }
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_output_stream(
                &sc,
                move |data: &mut [u16], _| {
                    let mut r = ring_cb.lock().unwrap();
                    for s in data.iter_mut() {
                        let v = r.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0);
                        *s = ((v * 0.5 + 0.5) * 65535.0) as u16;
                    }
                },
                err_fn,
                None,
            ),
            _ => return None,
        }
        .ok()?;
        stream.play().ok()?;

        Some(Self {
            _stream: stream,
            ring,
            sample_rate,
            channels,
            phase: 0.0,
        })
    }

    /// Recebe um lote de amostras do APU (i16 intercaladas L,R a `in_rate`),
    /// reamostra para a taxa do dispositivo (interpolação linear) e empurra no
    /// ring. Se o ring crescer demais (produtor mais rápido que o consumidor),
    /// descarta para ressincronizar.
    pub fn push(&mut self, apu: &[i16], in_rate: u32) {
        let frames = apu.len() / 2;
        if frames == 0 {
            return;
        }
        let step = in_rate as f64 / self.sample_rate as f64; // frames de entrada por frame de saída
        let mut out: Vec<f32> = Vec::new();

        while self.phase < frames as f64 {
            let i = self.phase.floor() as usize;
            let frac = (self.phase - i as f64) as f32;
            let l0 = apu[i * 2] as f32 / 32768.0;
            let r0 = apu[i * 2 + 1] as f32 / 32768.0;
            let (l1, r1) = if i + 1 < frames {
                (
                    apu[(i + 1) * 2] as f32 / 32768.0,
                    apu[(i + 1) * 2 + 1] as f32 / 32768.0,
                )
            } else {
                (l0, r0)
            };
            let l = l0 + (l1 - l0) * frac;
            let r = r0 + (r1 - r0) * frac;

            match self.channels {
                1 => out.push((l + r) * 0.5),
                _ => {
                    out.push(l);
                    out.push(r);
                    // Canais extras (surround) ficam em silêncio.
                    out.extend(std::iter::repeat_n(
                        0.0,
                        self.channels.saturating_sub(2) as usize,
                    ));
                }
            }
            self.phase += step;
        }
        self.phase -= frames as f64; // carrega a fração para o próximo lote

        let mut ring = self.ring.lock().unwrap();
        // Limita a latência a ~250 ms; se estourar, ressincroniza.
        let max = (self.sample_rate as usize) * (self.channels as usize) / 4;
        if ring.len() + out.len() > max {
            ring.clear();
        }
        ring.extend(out);
    }
}
