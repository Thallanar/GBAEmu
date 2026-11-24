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
//!
//! O trace também mede o **custo de cada atualização** (média/máximo em ms):
//! `emu` (frames do GBA), `upload` (upscale na CPU + envio da textura),
//! `update` (o `update` inteiro do egui, inclusive a UI) e `cpu` (o frame todo
//! segundo o eframe: `update` + tessellation + desenho, sem a espera do vsync).
//! Uma atualização que **perdeu vsync** (a seguinte veio > 1,5 período depois)
//! entra também numa segunda linha, `perdidos`, pra comparar o perfil delas com
//! o geral e achar o culpado.

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
/// Quantos intervalos recentes entram na estimativa do vsync (~1 s a 144 Hz).
const RECENT: usize = 120;
/// Percentil dos intervalos recentes tomado como o período do vsync. Perder
/// vsync só gera intervalos **longos**, então um percentil baixo continua no
/// período certo mesmo com até ~80% das atualizações atrasadas — e se ajusta
/// em ~1 s a uma mudança real (rajada sem vsync na abertura da janela, monitor
/// trocou de Hz).
const PERIOD_PERCENTILE: f64 = 0.2;
/// Monitor "rápido" (≥ 1,1 × GBA, ex.: 75/144 Hz em modo livre): sobram
/// atualizações vazias, então o atraso de um vsync perdido é pago nelas, 1
/// frame por vez, em vez de rodar 2 frames de uma vez — o que custa o dobro,
/// perde o vsync seguinte também e prende o app num ciclo (48 Hz em 144 Hz).
const FAST_DISPLAY: f64 = 1.1;
/// No monitor rápido, 2 frames na mesma atualização só com dívida ≥ isto
/// (o app realmente não está dando conta de 1 frame por atualização).
const CATCH_UP_DEBT: f64 = 2.5;

/// Decide quantos frames do GBA rodar em cada atualização de tela.
pub struct Pacer {
    last: Option<Instant>,
    /// Média móvel do intervalo entre vsyncs (s). `None` até a 1ª amostra.
    period: Option<f64>,
    /// Frames devidos (fração acumulada).
    acc: f64,
    /// `Some(n)`: travado em 1 frame a cada `n` vsyncs.
    lock: Option<u32>,
    /// Últimos [`RECENT`] intervalos entre atualizações (s).
    recent: std::collections::VecDeque<f64>,
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
            recent: std::collections::VecDeque::with_capacity(RECENT),
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

        // Estima o período do vsync: a base robusta é um percentil baixo dos
        // intervalos recentes; a média móvel (precisa, usada no `speed()` do
        // modo travado) só recebe intervalos perto dela. Atualizações que
        // perderam vsync ficam de fora e só contam como vários ticks.
        if self.recent.len() == RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(dt.max(1e-4));
        let base = self.base_period();
        let period = match self.period {
            Some(p) if (p - base).abs() <= base * 0.1 => {
                if (dt - base).abs() <= base * 0.25 {
                    p * 0.95 + dt * 0.05
                } else {
                    p
                }
            }
            // Sem estimativa, ou a média se afastou da base: recomeça dela.
            _ => base,
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

        let fast = self.lock.is_none() && period * GBA_FPS * FAST_DISPLAY < 1.0;
        let whole = self.acc.floor() as u32;
        let frames = if fast && whole >= 2 {
            // Monitor rápido: 1 frame por vez; 2 só com dívida grande.
            if self.acc >= CATCH_UP_DEBT {
                2
            } else {
                1
            }
        } else {
            whole.min(MAX_FRAMES)
        };
        self.acc -= frames as f64;
        // No monitor rápido a dívida (< 1,5 frame) fica pra próxima atualização.
        let keep = if fast { CATCH_UP_DEBT - 1.0 } else { 1.0 };
        if self.acc >= keep {
            // Atrasado demais: descarta o excesso em vez de rodar em rajada.
            self.acc = self.acc.fract();
        }
        frames
    }

    /// O [`PERIOD_PERCENTILE`] dos intervalos recentes.
    fn base_period(&self) -> f64 {
        let mut v: Vec<f64> = self.recent.iter().copied().collect();
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * PERIOD_PERCENTILE) as usize]
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

/// Custo (s) das partes de uma atualização de tela, medido no `update`.
#[derive(Clone, Copy, Default)]
pub struct UpdateCost {
    /// Frames do GBA rodados (`step_frame`).
    pub emu: f64,
    /// Upscale na CPU + envio da textura pro GL.
    pub upload: f64,
    /// O `update` inteiro do egui (inclui emu, upload e a montagem da UI).
    pub update: f64,
}

/// Soma/máximo de uma métrica numa janela do trace.
#[derive(Clone, Copy, Default)]
struct Stat {
    sum: f64,
    max: f64,
}

impl Stat {
    fn add(&mut self, v: f64) {
        self.sum += v;
        self.max = self.max.max(v);
    }

    /// "média/máx" em ms sobre `n` amostras.
    fn fmt(&self, n: u32) -> String {
        if n == 0 {
            return "—".into();
        }
        format!("{:.2}/{:.2}", self.sum / n as f64 * 1e3, self.max * 1e3)
    }
}

/// Custos acumulados (emu, upload, update, cpu) de um grupo de atualizações.
#[derive(Clone, Copy, Default)]
struct CostStats {
    n: u32,
    /// Quantas dessas atualizações rodaram ≥1 frame do GBA.
    with_frame: u32,
    emu: Stat,
    upload: Stat,
    update: Stat,
    cpu: Stat,
}

impl CostStats {
    fn add(&mut self, c: &UpdateCost, frames: u32, cpu: Option<f64>) {
        self.n += 1;
        if frames > 0 {
            self.with_frame += 1;
        }
        self.emu.add(c.emu);
        self.upload.add(c.upload);
        self.update.add(c.update);
        if let Some(cpu) = cpu {
            self.cpu.add(cpu);
        }
    }

    fn fmt(&self) -> String {
        format!(
            "{} (c/ frame {}) · emu {} · upload {} · update {} · cpu {}",
            self.n,
            self.with_frame,
            self.emu.fmt(self.n),
            self.upload.fmt(self.n),
            self.update.fmt(self.n),
            self.cpu.fmt(self.n),
        )
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
    /// Custo e frames da atualização anterior (fechada por [`Self::end_update`]).
    prev: Option<(UpdateCost, u32)>,
    /// Frames rodados na atualização corrente (do `record`).
    cur_frames: u32,
    /// `record` rodou nesta atualização (então `end_update` deve fechá-la).
    recorded: bool,
    /// Custos de todas as atualizações da janela e só das que perderam vsync.
    cost_all: CostStats,
    cost_missed: CostStats,
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
            prev: None,
            cur_frames: 0,
            recorded: false,
            cost_all: CostStats::default(),
            cost_missed: CostStats::default(),
        })
    }

    /// Registra uma atualização de tela que rodou `frames` frames do GBA.
    /// `audio_ms` é a fila de áudio no fim da atualização (se houver áudio).
    /// `prev_cpu` é o `cpu_usage` do eframe (s): o custo do frame **anterior**.
    pub fn record(
        &mut self,
        now: Instant,
        frames: u32,
        pacer: &Pacer,
        audio_ms: Option<f64>,
        prev_cpu: Option<f64>,
    ) {
        if let Some(last) = self.last_update {
            let dt = now.duration_since(last).as_secs_f64();
            self.dt_sum += dt;
            // O intervalo até agora diz se a atualização anterior perdeu vsync;
            // os custos dela (e o cpu_usage, que é dela) vão pro grupo certo.
            if let Some((cost, f)) = self.prev.take() {
                self.cost_all.add(&cost, f, prev_cpu);
                let missed = pacer.display_hz().is_some_and(|hz| dt * hz > 1.5);
                if missed {
                    self.cost_missed.add(&cost, f, prev_cpu);
                }
            }
        } else {
            self.prev = None;
        }
        self.cur_frames = frames;
        self.recorded = true;
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
            eprintln!(
                "  custo ms (média/máx): todas {}\n  custo ms (média/máx): perdidos {}",
                self.cost_all.fmt(),
                self.cost_missed.fmt(),
            );
            self.cost_all = CostStats::default();
            self.cost_missed = CostStats::default();
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

impl PacingTrace {
    /// Fecha a atualização corrente com os custos medidos. Só vale depois de um
    /// [`Self::record`] na mesma atualização (ritmo normal); senão é ignorado.
    pub fn end_update(&mut self, cost: UpdateCost) {
        if std::mem::take(&mut self.recorded) {
            self.prev = Some((cost, self.cur_frames));
        } else {
            self.prev = None;
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

    /// Em 144 Hz, se toda atualização que roda um frame perde 1 vsync (dura 2),
    /// o atraso é pago nas vazias: nunca 2 frames juntos, velocidade = GBA, e a
    /// estimativa do monitor continua em 144 Hz.
    #[test]
    fn em_144hz_vsync_perdido_nao_vira_rajada() {
        let mut p = Pacer::new();
        let start = Instant::now();
        let period = Duration::from_secs_f64(1.0 / 144.0);
        let mut t = start;
        for _ in 0..300 {
            p.tick(t);
            t += period;
        }
        let (mut frames, mut doubles) = (0, 0);
        let t0 = t;
        while t - t0 < Duration::from_secs(10) {
            let f = p.tick(t);
            frames += f;
            doubles += (f >= 2) as u32;
            t += period * if f > 0 { 2 } else { 1 };
        }
        assert_eq!(doubles, 0);
        let fps = frames as f64 / 10.0;
        assert!((fps - GBA_FPS).abs() < 0.5, "{fps}");
        assert!((p.display_hz().unwrap() - 144.0).abs() < 1.0);
    }

    /// Se nem 1 frame por atualização cabe (toda atualização dura 3 vsyncs = 48
    /// Hz efetivos), o pacer ainda roda 2 frames de vez em quando pra manter a
    /// velocidade do jogo.
    #[test]
    fn em_144hz_app_lento_ainda_mantem_a_velocidade() {
        let mut p = Pacer::new();
        let start = Instant::now();
        let period = Duration::from_secs_f64(1.0 / 144.0);
        let mut t = start;
        for _ in 0..300 {
            p.tick(t);
            t += period;
        }
        let mut frames = 0;
        let t0 = t;
        while t - t0 < Duration::from_secs(4) {
            frames += p.tick(t);
            t += period * 3;
        }
        let fps = frames as f64 / 4.0;
        assert!((fps - GBA_FPS).abs() < 1.0, "{fps}");
    }

    /// Na abertura da janela o app pode rodar atualizações sem vsync (rápidas
    /// demais); a estimativa não pode ficar presa nelas.
    #[test]
    fn rajada_sem_vsync_na_abertura_nao_engana_a_estimativa() {
        let mut p = Pacer::new();
        let mut t = Instant::now();
        for _ in 0..300 {
            p.tick(t);
            t += Duration::from_secs_f64(1.0 / 400.0);
        }
        for _ in 0..288 {
            p.tick(t);
            t += Duration::from_secs_f64(1.0 / 144.0);
        }
        let hz = p.display_hz().unwrap();
        assert!((hz - 144.0).abs() < 1.0, "{hz}");
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
