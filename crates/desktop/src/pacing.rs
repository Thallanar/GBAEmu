//! Pacing do desktop (Fase 9, etapa 2): quando rodar cada frame do GBA.
//!
//! O ritmo vem do **monitor** (cada `update` do egui = 1 vsync), não do áudio.
//! Pacing pelo áudio entregava rajadas: o sistema puxa o som em blocos de
//! dezenas de ms, então a emulação rodava 2–3 frames de uma vez e ficava várias
//! atualizações parada — só ~24 imagens distintas por segundo chegavam à tela.
//!
//! [`Pacer`] estima a frequência do monitor e:
//! - se ela é ~múltiplo inteiro de 59,73 Hz (60, 120, 59,94…), **trava** em
//!   1 frame a cada N vsyncs — cadência perfeita; o jogo roda a `Hz/N` (0,46%
//!   mais rápido em 60 Hz), e o áudio absorve a diferença ([`Pacer::speed`]);
//! - senão (75, 144 Hz…), distribui os frames o mais uniforme possível pelos
//!   vsyncs (em 144 Hz: alterna 2 e 3 vsyncs por frame, sem rajadas).
//!
//! O áudio ([`crate::audio::AudioOut`]) reamostra com a velocidade do pacer e
//! corrige a deriva entre os relógios do monitor e da placa de som (dynamic
//! rate control, ±0,5%).
//!
//! O trace liga com `AURORAGBA_PACING_TRACE=1`: a cada ~1 s imprime no stderr quantas
//! atualizações de tela houve, a frequência estimada do monitor, quantos frames
//! do GBA cada atualização rodou e — a métrica do judder — o intervalo (em
//! atualizações de tela) entre um frame novo e o seguinte. Num ritmo uniforme o
//! histograma de intervalos fica concentrado (ex.: só 2 em 120 Hz; só 2 e 3 em
//! 144 Hz); rajadas aparecem como intervalos 0 (2+ frames na mesma atualização)
//! e saltos grandes.

use std::time::Instant;

/// Frames por segundo do GBA real (16,78 MHz / 280.896 ciclos por frame).
pub const GBA_FPS: f64 = 59.7275;
/// Erro máximo (relativo) entre `Hz/N` e o GBA para travar em 1 frame/N vsyncs.
const LOCK_ENTER: f64 = 0.010;
/// Histerese: só destrava se o erro passar disto (evita oscilar na fronteira).
const LOCK_EXIT: f64 = 0.020;
/// Maior N travável (até ~240 Hz).
const MAX_LOCK_N: u32 = 4;
/// Teto de frames por atualização (UI travou um tempo: não tenta recuperar tudo).
const MAX_FRAMES: u32 = 4;
/// Pausa acima disto (janela minimizada, diálogo…) reinicia o pacing.
const RESYNC_GAP: f64 = 0.25;

/// Decide quantos frames do GBA rodar em cada atualização de tela.
pub struct Pacer {
    last: Option<Instant>,
    /// Média móvel do intervalo entre vsyncs (s). `None` até a 1ª amostra.
    period: Option<f64>,
    /// Frames devidos (fração acumulada).
    acc: f64,
    /// `Some(n)`: travado em 1 frame a cada `n` vsyncs.
    lock: Option<u32>,
    /// Intervalos seguidos descartados como fora da média. Muitos seguidos =
    /// a média é que está errada (ex.: 1ª amostra ruim, monitor trocou de Hz).
    outliers: u32,
}

impl Default for Pacer {
    fn default() -> Self {
        Self::new()
    }
}

impl Pacer {
    pub fn new() -> Self {
        Self {
            last: None,
            period: None,
            acc: 0.0,
            lock: None,
            outliers: 0,
        }
    }

    /// Esquece a fase (depois de fast-forward, rewind, pausa…): a próxima
    /// atualização roda 1 frame e o ritmo recomeça dali. A estimativa do
    /// monitor é mantida.
    pub fn reset(&mut self) {
        self.last = None;
        self.acc = 0.0;
    }

    /// Frequência estimada do monitor (Hz), se já houver amostras.
    pub fn display_hz(&self) -> Option<f64> {
        self.period.map(|p| 1.0 / p)
    }

    /// `Some(n)` se travado em 1 frame a cada `n` vsyncs.
    pub fn lock(&self) -> Option<u32> {
        self.lock
    }

    /// Velocidade da emulação relativa ao GBA real (1,0 = exata). Só difere de
    /// 1 quando travado: em 60 Hz é 60/59,7275 ≈ 1,0046. O áudio usa isto para
    /// reamostrar sem mudar o tamanho da fila.
    pub fn speed(&self) -> f64 {
        match (self.lock, self.period) {
            (Some(n), Some(p)) => 1.0 / (p * n as f64 * GBA_FPS),
            _ => 1.0,
        }
    }

    /// Chamado uma vez por atualização de tela; devolve quantos frames rodar.
    pub fn tick(&mut self, now: Instant) -> u32 {
        let Some(last) = self.last.replace(now) else {
            self.acc = 0.0;
            return 1;
        };
        let dt = now.duration_since(last).as_secs_f64();
        if dt > RESYNC_GAP {
            self.acc = 0.0;
            return 1;
        }

        // Estima o período do vsync. Atualizações que perderam vsync (dt bem
        // maior que a média) não entram na média, só contam como vários ticks.
        let period = match self.period {
            Some(p) if (dt > p * 1.5 || dt < p * 0.5) && self.outliers < 30 => {
                self.outliers += 1;
                p
            }
            Some(p) if self.outliers < 30 => {
                self.outliers = 0;
                p * 0.95 + dt * 0.05
            }
            _ => {
                self.outliers = 0;
                dt.max(1e-4)
            }
        };
        self.period = Some(period);
        self.update_lock(period);

        self.acc += match self.lock {
            // Travado: conta vsyncs (1 normalmente; mais se a UI atrasou) — a
            // cadência fica exata e a velocidade sai de `speed()`.
            Some(n) => (dt / period).round().clamp(1.0, MAX_FRAMES as f64) / n as f64,
            // Livre: tempo real decorrido, para a velocidade média ser a do GBA
            // mesmo se o app perder vsyncs (o que enviesa a média do período).
            None => dt * GBA_FPS,
        };

        let frames = (self.acc.floor() as u32).min(MAX_FRAMES);
        self.acc -= frames as f64;
        if self.acc >= 1.0 {
            // Atrasado demais: descarta o excesso em vez de rodar em rajada.
            self.acc = self.acc.fract();
        }
        frames
    }

    fn update_lock(&mut self, period: f64) {
        let hz = 1.0 / period;
        let err = |n: u32| (hz / (n as f64 * GBA_FPS) - 1.0).abs();
        if let Some(n) = self.lock {
            if err(n) <= LOCK_EXIT {
                return;
            }
        }
        let n = (hz / GBA_FPS).round() as u32;
        self.lock = (1..=MAX_LOCK_N)
            .contains(&n)
            .then_some(n)
            .filter(|&n| err(n) <= LOCK_ENTER);
    }
}

/// Variável de ambiente que liga o trace.
pub const TRACE_ENV: &str = "AURORAGBA_PACING_TRACE";

pub struct PacingTrace {
    window_start: Instant,
    last_update: Option<Instant>,
    updates: u32,
    dt_sum: f64,
    frames: u32,
    /// Atualizações com 0, 1, 2 e 3+ frames rodados.
    per_update: [u32; 4],
    /// Atualizações desde o último frame novo.
    since_frame: u32,
    /// Intervalos entre frames: 0 (mesma atualização), 1, 2, 3, 4, 5+.
    gaps: [u32; 6],
    /// Fila de áudio (ms) mínima/máxima na janela.
    audio_ms: Option<(f64, f64)>,
}

impl PacingTrace {
    /// `Some` só se a variável de ambiente estiver ligada.
    pub fn from_env() -> Option<Self> {
        std::env::var_os(TRACE_ENV)?;
        Some(Self {
            window_start: Instant::now(),
            last_update: None,
            updates: 0,
            dt_sum: 0.0,
            frames: 0,
            per_update: [0; 4],
            since_frame: 0,
            gaps: [0; 6],
            audio_ms: None,
        })
    }

    /// Registra uma atualização de tela que rodou `frames` frames do GBA.
    /// `audio_ms` é a fila de áudio no fim da atualização (se houver áudio).
    pub fn record(&mut self, now: Instant, frames: u32, pacer: &Pacer, audio_ms: Option<f64>) {
        if let Some(last) = self.last_update {
            self.dt_sum += now.duration_since(last).as_secs_f64();
        }
        self.last_update = Some(now);
        self.updates += 1;
        self.frames += frames;
        self.per_update[(frames as usize).min(3)] += 1;

        self.since_frame += 1;
        if frames > 0 {
            self.gaps[(self.since_frame as usize).min(5)] += 1;
            self.gaps[0] += frames - 1;
            self.since_frame = 0;
        }

        if let Some(ms) = audio_ms {
            self.audio_ms = Some(match self.audio_ms {
                Some((lo, hi)) => (lo.min(ms), hi.max(ms)),
                None => (ms, ms),
            });
        }

        let elapsed = now.duration_since(self.window_start).as_secs_f64();
        if elapsed >= 1.0 {
            let hz = if self.updates > 1 {
                (self.updates - 1) as f64 / self.dt_sum
            } else {
                0.0
            };
            let mode = match (pacer.lock(), pacer.display_hz()) {
                (Some(n), Some(hz)) => format!("travado 1/{n} @ {hz:.2} Hz"),
                (None, Some(hz)) => format!("livre @ {hz:.2} Hz"),
                _ => "—".into(),
            };
            let audio = match self.audio_ms {
                Some((lo, hi)) => format!("áudio {lo:.0}–{hi:.0} ms"),
                None => "sem áudio".into(),
            };
            eprintln!(
                "pacing: {:.1} fps · {} updates (~{hz:.1} Hz) · frames/update [0:{} 1:{} 2:{} 3+:{}] · intervalo [0:{} 1:{} 2:{} 3:{} 4:{} 5+:{}] · {mode} · {audio}",
                self.frames as f64 / elapsed,
                self.updates,
                self.per_update[0],
                self.per_update[1],
                self.per_update[2],
                self.per_update[3],
                self.gaps[0],
                self.gaps[1],
                self.gaps[2],
                self.gaps[3],
                self.gaps[4],
                self.gaps[5],
            );
            self.window_start = now;
            self.updates = 0;
            self.dt_sum = 0.0;
            self.frames = 0;
            self.per_update = [0; 4];
            self.gaps = [0; 6];
            self.audio_ms = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Simula `n` vsyncs a `hz` e devolve os frames rodados em cada um.
    fn simulate(hz: f64, n: usize) -> (Pacer, Vec<u32>) {
        let mut p = Pacer::new();
        let start = Instant::now();
        let period = Duration::from_secs_f64(1.0 / hz);
        let frames = (0..n).map(|i| p.tick(start + period * i as u32)).collect();
        (p, frames)
    }

    #[test]
    fn trava_em_60hz_com_um_frame_por_vsync() {
        let (p, frames) = simulate(60.0, 600);
        assert_eq!(p.lock(), Some(1));
        assert!(frames.iter().all(|&f| f == 1));
        assert!((p.speed() - 60.0 / GBA_FPS).abs() < 1e-6);
    }

    #[test]
    fn trava_em_120hz_alternando_frame_e_vazio() {
        let (p, frames) = simulate(120.0, 600);
        assert_eq!(p.lock(), Some(2));
        // Depois de assentar, padrão estrito 1,0,1,0…
        let tail = &frames[100..];
        assert!(tail.windows(2).all(|w| w[0] + w[1] == 1));
    }

    #[test]
    fn em_144hz_nao_trava_e_espaca_os_frames_por_2_ou_3_vsyncs() {
        let (p, frames) = simulate(144.0, 1440);
        assert_eq!(p.lock(), None);
        assert!(frames.iter().all(|&f| f <= 1));
        let hits: Vec<usize> = frames
            .iter()
            .enumerate()
            .filter(|(_, &f)| f == 1)
            .map(|(i, _)| i)
            .collect();
        assert!(hits
            .windows(2)
            .skip(5)
            .all(|w| matches!(w[1] - w[0], 2 | 3)));
        // Velocidade média = GBA real.
        let fps = frames.iter().sum::<u32>() as f64 / 10.0;
        assert!((fps - GBA_FPS).abs() < 0.2, "{fps}");
    }

    #[test]
    fn vsync_perdido_conta_como_dois_ticks() {
        let mut p = Pacer::new();
        let t0 = Instant::now();
        let period = Duration::from_secs_f64(1.0 / 60.0);
        for i in 0..120 {
            p.tick(t0 + period * i);
        }
        // Pula um vsync: a próxima atualização compensa com 2 frames.
        assert_eq!(p.tick(t0 + period * 121), 2);
        assert_eq!(p.lock(), Some(1));
    }

    #[test]
    fn pausa_longa_recomeca_sem_rajada() {
        let mut p = Pacer::new();
        let t0 = Instant::now();
        let period = Duration::from_secs_f64(1.0 / 60.0);
        for i in 0..60 {
            p.tick(t0 + period * i);
        }
        assert_eq!(p.tick(t0 + Duration::from_secs(5)), 1);
    }

    #[test]
    fn primeira_amostra_ruim_se_corrige() {
        let mut p = Pacer::new();
        let t0 = Instant::now();
        // 1º intervalo de 40 ms (janela abrindo), depois 144 Hz estável.
        p.tick(t0);
        let mut t = t0 + Duration::from_millis(40);
        p.tick(t);
        for _ in 0..200 {
            t += Duration::from_secs_f64(1.0 / 144.0);
            p.tick(t);
        }
        let hz = p.display_hz().unwrap();
        assert!((hz - 144.0).abs() < 1.0, "{hz}");
    }
}
