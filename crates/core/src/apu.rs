//! APU — Audio Processing Unit do GBA.
//!
//! São 6 canais somados num mixer. Os 4 canais PSG herdados do Game Boy (este
//! arquivo): (1) onda quadrada com sweep de frequência, (2) onda quadrada,
//! (3) wave (32 amostras de 4 bits em RAM programável), (4) ruído (LFSR de
//! 15/7 bits). Os 2 canais Direct Sound (PCM 8-bit via FIFO + Timer + DMA)
//! ficam para etapa futura.
//!
//! Os canais PSG são clocados a 4.194304 MHz (= ciclos da CPU / 4). Um
//! "frame sequencer" a 512 Hz governa length (256 Hz), envelope (64 Hz) e
//! sweep (128 Hz). A saída é reamostrada para [`OUTPUT_RATE`] num ring buffer
//! que o frontend drena para a placa de som.
//!
//! Referência: GBATEK "GBA Sound" + Pan Docs (APU do Game Boy).

/// Registradores de som mapeados em 0x04000060..0x040000A8.
pub const SOUND_BASE: u32 = 0x0400_0060;
pub const SOUND_END: u32 = 0x0400_00A8;

/// Taxa de amostragem da saída para o host.
pub const OUTPUT_RATE: u32 = 32_768;
/// Clock do APU (base do Game Boy): CPU (16.78 MHz) / 4.
const APU_CLOCK: u32 = 4_194_304;
/// Ciclos de APU por amostra de saída.
const CYCLES_PER_SAMPLE: u32 = APU_CLOCK / OUTPUT_RATE; // 128

/// Padrões de duty da onda quadrada (8 passos, 1 = alto).
const DUTY: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1], // 25%
    [1, 0, 0, 0, 0, 1, 1, 1], // 50%
    [0, 1, 1, 1, 1, 1, 1, 0], // 75%
];

#[derive(Default)]
pub struct Apu {
    /// Master enable (SOUNDCNT_X / NR52 bit 7).
    enabled: bool,
    /// SOUNDCNT_L (0x80): volume e pan dos PSG.
    cnt_l: u16,
    /// SOUNDCNT_H (0x82): mix dos PSG + controle do Direct Sound.
    cnt_h: u16,
    /// SOUNDBIAS (0x88).
    bias: u16,

    ch1: Square,
    ch2: Square,
    ch3: Wave,
    ch4: Noise,

    /// Contador de ciclos do frame sequencer (passo a cada 8192 ciclos = 512 Hz).
    seq_cycles: u32,
    seq_step: u8,
    /// Acumulador para reamostragem para [`OUTPUT_RATE`].
    sample_cycles: u32,
    /// Frações de ciclo de CPU não consumidas (CPU roda a 4× o APU).
    cpu_frac: u32,

    /// Ring buffer de saída, intercalado L,R,L,R... em i16.
    pub buffer: Vec<i16>,
}

impl Apu {
    pub fn new() -> Self {
        Self::default()
    }

    // ───────────────────────── I/O de registradores ─────────────────────────

    pub fn read_u8(&self, addr: u32) -> u8 {
        match addr {
            0x0400_0080 => self.cnt_l as u8,
            0x0400_0081 => (self.cnt_l >> 8) as u8,
            0x0400_0082 => self.cnt_h as u8,
            0x0400_0083 => (self.cnt_h >> 8) as u8,
            // SOUNDCNT_X (0x84): bit7 master enable; bits0-3 status dos canais.
            0x0400_0084 => {
                ((self.enabled as u8) << 7)
                    | (self.ch1.on as u8)
                    | ((self.ch2.on as u8) << 1)
                    | ((self.ch3.on as u8) << 2)
                    | ((self.ch4.on as u8) << 3)
            }
            0x0400_0088 => self.bias as u8,
            0x0400_0089 => (self.bias >> 8) as u8,
            // Wave RAM (0x90..0x9F).
            0x0400_0090..=0x0400_009F => self.ch3.read_ram(addr - 0x0400_0090),
            _ => 0,
        }
    }

    pub fn write_u8(&mut self, addr: u32, val: u8) {
        // Com o master desligado, só SOUNDCNT_X (0x84) e a wave RAM respondem.
        if !self.enabled && addr != 0x0400_0084 && !(0x0400_0090..=0x0400_009F).contains(&addr) {
            return;
        }
        // O mapa de som do GBA tem gaps; despachamos por endereço para "campos
        // lógicos" (0=sweep/dac/len, 1=duty/len/env, 2=env/vol, 3=freqlo, 4=freqhi).
        match addr {
            // Canal 1 (quadrada + sweep)
            0x0400_0060 => self.ch1.write_reg(0, val), // NR10 sweep
            0x0400_0062 => self.ch1.write_reg(1, val), // NR11 duty/length
            0x0400_0063 => self.ch1.write_reg(2, val), // NR12 envelope
            0x0400_0064 => self.ch1.write_reg(3, val), // NR13 freq baixa
            0x0400_0065 => self.ch1.write_reg(4, val), // NR14 freq alta/trigger
            // Canal 2 (quadrada)
            0x0400_0068 => self.ch2.write_reg(1, val), // NR21 duty/length
            0x0400_0069 => self.ch2.write_reg(2, val), // NR22 envelope
            0x0400_006C => self.ch2.write_reg(3, val), // NR23 freq baixa
            0x0400_006D => self.ch2.write_reg(4, val), // NR24 freq alta/trigger
            // Canal 3 (wave)
            0x0400_0070 => self.ch3.write_reg(0, val), // NR30 DAC
            0x0400_0072 => self.ch3.write_reg(1, val), // NR31 length
            0x0400_0073 => self.ch3.write_reg(2, val), // NR32 volume
            0x0400_0074 => self.ch3.write_reg(3, val), // NR33 freq baixa
            0x0400_0075 => self.ch3.write_reg(4, val), // NR34 freq alta/trigger
            // Canal 4 (ruído)
            0x0400_0078 => self.ch4.write_reg(0, val), // NR41 length
            0x0400_0079 => self.ch4.write_reg(1, val), // NR42 envelope
            0x0400_007C => self.ch4.write_reg(2, val), // NR43 freq/divisor
            0x0400_007D => self.ch4.write_reg(3, val), // NR44 trigger/length-enable
            0x0400_0080 => self.cnt_l = (self.cnt_l & 0xFF00) | val as u16,
            0x0400_0081 => self.cnt_l = (self.cnt_l & 0x00FF) | ((val as u16) << 8),
            0x0400_0082 => self.cnt_h = (self.cnt_h & 0xFF00) | val as u16,
            0x0400_0083 => self.cnt_h = (self.cnt_h & 0x00FF) | ((val as u16) << 8),
            0x0400_0084 => {
                let was = self.enabled;
                self.enabled = val & 0x80 != 0;
                // Desligar o master zera todos os canais (como no Game Boy).
                if was && !self.enabled {
                    self.ch1 = Square::default();
                    self.ch2 = Square::default();
                    self.ch3 = Wave::default();
                    self.ch4 = Noise::default();
                }
            }
            0x0400_0088 => self.bias = (self.bias & 0xFF00) | val as u16,
            0x0400_0089 => self.bias = (self.bias & 0x00FF) | ((val as u16) << 8),
            0x0400_0090..=0x0400_009F => self.ch3.write_ram(addr - 0x0400_0090, val),
            _ => {}
        }
    }

    // ───────────────────────────── Avanço ───────────────────────────────────

    /// Avança o APU por `cpu_cycles` ciclos de CPU, gerando amostras no buffer.
    pub fn tick(&mut self, cpu_cycles: u32) {
        // A CPU roda a 4× o clock do APU.
        let total = cpu_cycles + self.cpu_frac;
        let apu_cycles = total / 4;
        self.cpu_frac = total % 4;

        for _ in 0..apu_cycles {
            self.ch1.tick();
            self.ch2.tick();
            self.ch3.tick();
            self.ch4.tick();

            // Frame sequencer a 512 Hz.
            self.seq_cycles += 1;
            if self.seq_cycles >= APU_CLOCK / 512 {
                self.seq_cycles = 0;
                self.step_sequencer();
            }

            // Reamostragem para a saída.
            self.sample_cycles += 1;
            if self.sample_cycles >= CYCLES_PER_SAMPLE {
                self.sample_cycles = 0;
                let (l, r) = self.mix();
                self.buffer.push(l);
                self.buffer.push(r);
            }
        }
    }

    fn step_sequencer(&mut self) {
        // Passos 0..7: length em 0,2,4,6; sweep em 2,6; envelope em 7.
        if self.seq_step.is_multiple_of(2) {
            self.ch1.clock_length();
            self.ch2.clock_length();
            self.ch3.clock_length();
            self.ch4.clock_length();
        }
        if self.seq_step == 2 || self.seq_step == 6 {
            self.ch1.clock_sweep();
        }
        if self.seq_step == 7 {
            self.ch1.clock_envelope();
            self.ch2.clock_envelope();
            self.ch4.clock_envelope();
        }
        self.seq_step = (self.seq_step + 1) & 7;
    }

    /// Mixa os 4 canais PSG em L/R aplicando os pans/volumes de SOUNDCNT_L.
    fn mix(&self) -> (i16, i16) {
        if !self.enabled {
            return (0, 0);
        }
        let s = [
            self.ch1.sample() as i32,
            self.ch2.sample() as i32,
            self.ch3.sample() as i32,
            self.ch4.sample() as i32,
        ];
        // SOUNDCNT_L: bits0-2 vol R, bits4-6 vol L, bits8-11 enable R, 12-15 enable L.
        let vol_r = (self.cnt_l & 0x07) as i32;
        let vol_l = ((self.cnt_l >> 4) & 0x07) as i32;
        let en_r = (self.cnt_l >> 8) & 0x0F;
        let en_l = (self.cnt_l >> 12) & 0x0F;

        let mut l = 0i32;
        let mut r = 0i32;
        for (i, &v) in s.iter().enumerate() {
            if en_l & (1 << i) != 0 {
                l += v;
            }
            if en_r & (1 << i) != 0 {
                r += v;
            }
        }
        // Cada canal dá -15..+15; 4 canais → ±60. Escala por volume (0..7) e por
        // um ganho fixo para aproveitar a faixa de i16.
        l = l * (vol_l + 1) * 80;
        r = r * (vol_r + 1) * 80;
        (l.clamp(-32768, 32767) as i16, r.clamp(-32768, 32767) as i16)
    }

    /// Remove e devolve todas as amostras acumuladas (chamado pelo frontend).
    pub fn drain(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.buffer)
    }
}

// ───────────────────────── Envelope (canais 1,2,4) ─────────────────────────

#[derive(Default)]
struct Envelope {
    initial: u8, // volume inicial 0..15
    add: bool,   // true = aumenta
    period: u8,  // passos do frame sequencer entre mudanças (0 = parado)
    volume: u8,  // volume atual
    timer: u8,
}

impl Envelope {
    fn trigger(&mut self) {
        self.volume = self.initial;
        self.timer = self.period;
    }
    fn clock(&mut self) {
        if self.period == 0 {
            return;
        }
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.timer = self.period;
            if self.add && self.volume < 15 {
                self.volume += 1;
            } else if !self.add && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }
}

// ───────────────────────── Canal de onda quadrada ─────────────────────────

#[derive(Default)]
struct Square {
    on: bool,
    duty: u8,
    duty_step: u8,
    freq: u16, // 11 bits
    timer: u16,
    length: u16, // contador de length (0..64)
    length_on: bool,
    env: Envelope,

    // Sweep (só canal 1).
    sweep_period: u8,
    sweep_neg: bool,
    sweep_shift: u8,
    sweep_timer: u8,
    sweep_enabled: bool,
    sweep_shadow: u16,
}

impl Square {
    fn period(&self) -> u16 {
        (2048 - (self.freq & 0x7FF)).wrapping_mul(4)
    }

    fn tick(&mut self) {
        if self.timer == 0 {
            self.timer = self.period();
            self.duty_step = (self.duty_step + 1) & 7;
        } else {
            self.timer -= 1;
        }
    }

    fn sample(&self) -> i8 {
        if !self.on {
            return 0;
        }
        let amp = DUTY[self.duty as usize][self.duty_step as usize];
        // 0/1 → ±volume.
        if amp != 0 {
            self.env.volume as i8
        } else {
            -(self.env.volume as i8)
        }
    }

    fn clock_length(&mut self) {
        if self.length_on && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.on = false;
            }
        }
    }

    fn clock_envelope(&mut self) {
        self.env.clock();
    }

    fn clock_sweep(&mut self) {
        if !self.sweep_enabled || self.sweep_period == 0 {
            return;
        }
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }
        if self.sweep_timer == 0 {
            self.sweep_timer = self.sweep_period;
            let new = self.sweep_calc();
            if new <= 2047 && self.sweep_shift > 0 {
                self.sweep_shadow = new;
                self.freq = new;
            } else if new > 2047 {
                self.on = false;
            }
        }
    }

    fn sweep_calc(&self) -> u16 {
        let delta = self.sweep_shadow >> self.sweep_shift;
        if self.sweep_neg {
            self.sweep_shadow.wrapping_sub(delta)
        } else {
            self.sweep_shadow + delta
        }
    }

    fn trigger(&mut self) {
        self.on = true;
        if self.length == 0 {
            self.length = 64;
        }
        self.timer = self.period();
        self.env.trigger();
        // Sweep.
        self.sweep_shadow = self.freq;
        self.sweep_timer = if self.sweep_period > 0 {
            self.sweep_period
        } else {
            8
        };
        self.sweep_enabled = self.sweep_period > 0 || self.sweep_shift > 0;
        if self.env.initial == 0 && !self.env.add {
            self.on = false; // DAC desligado
        }
    }

    /// Campo lógico: 0=sweep, 1=duty/length, 2=envelope, 3=freq baixa,
    /// 4=freq alta/trigger.
    fn write_reg(&mut self, field: u8, val: u8) {
        match field {
            0 => {
                // NR10 (sweep): bits0-2 shift, bit3 negate, bits4-6 period.
                self.sweep_shift = val & 0x07;
                self.sweep_neg = val & 0x08 != 0;
                self.sweep_period = (val >> 4) & 0x07;
            }
            1 => {
                // NRx1: bits0-5 length, bits6-7 duty.
                self.length = 64 - (val & 0x3F) as u16;
                self.duty = val >> 6;
            }
            2 => {
                // NRx2 (envelope): bits0-2 period, bit3 add, bits4-7 volume inicial.
                self.env.period = val & 0x07;
                self.env.add = val & 0x08 != 0;
                self.env.initial = val >> 4;
                if self.env.initial == 0 && !self.env.add {
                    self.on = false;
                }
            }
            3 => self.freq = (self.freq & 0x700) | val as u16, // freq baixa
            4 => {
                // bits0-2 freq alta, bit6 length enable, bit7 trigger.
                self.freq = (self.freq & 0x0FF) | (((val & 0x07) as u16) << 8);
                self.length_on = val & 0x40 != 0;
                if val & 0x80 != 0 {
                    self.trigger();
                }
            }
            _ => {}
        }
    }
}

// ───────────────────────────── Canal wave ─────────────────────────────────

#[derive(Default)]
struct Wave {
    on: bool,
    dac_on: bool,
    freq: u16,
    timer: u16,
    pos: u8, // 0..31
    volume_code: u8,
    length: u16,
    length_on: bool,
    ram: [u8; 16], // 32 amostras de 4 bits
}

impl Wave {
    fn tick(&mut self) {
        if self.timer == 0 {
            self.timer = (2048 - (self.freq & 0x7FF)).wrapping_mul(2);
            self.pos = (self.pos + 1) & 31;
        } else {
            self.timer -= 1;
        }
    }

    fn sample(&self) -> i8 {
        if !self.on || !self.dac_on {
            return 0;
        }
        let byte = self.ram[(self.pos / 2) as usize];
        let nib = if self.pos & 1 == 0 {
            byte >> 4
        } else {
            byte & 0xF
        };
        let shift = match self.volume_code {
            0 => 4, // mudo
            1 => 0, // 100%
            2 => 1, // 50%
            _ => 2, // 25%
        };
        // nibble 0..15 → centrado em ±7.
        ((nib >> shift) as i8) - 7
    }

    fn clock_length(&mut self) {
        if self.length_on && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.on = false;
            }
        }
    }

    fn trigger(&mut self) {
        self.on = self.dac_on;
        if self.length == 0 {
            self.length = 256;
        }
        self.timer = (2048 - (self.freq & 0x7FF)).wrapping_mul(2);
        self.pos = 0;
    }

    /// Campo lógico: 0=DAC, 1=length, 2=volume, 3=freq baixa, 4=freq alta/trigger.
    fn write_reg(&mut self, field: u8, val: u8) {
        match field {
            0 => self.dac_on = val & 0x80 != 0,
            1 => self.length = 256 - val as u16,
            2 => self.volume_code = (val >> 5) & 0x03,
            3 => self.freq = (self.freq & 0x700) | val as u16,
            4 => {
                self.freq = (self.freq & 0x0FF) | (((val & 0x07) as u16) << 8);
                self.length_on = val & 0x40 != 0;
                if val & 0x80 != 0 {
                    self.trigger();
                }
            }
            _ => {}
        }
    }

    fn read_ram(&self, off: u32) -> u8 {
        self.ram[off as usize]
    }
    fn write_ram(&mut self, off: u32, val: u8) {
        self.ram[off as usize] = val;
    }
}

// ───────────────────────────── Canal de ruído ──────────────────────────────

#[derive(Default)]
struct Noise {
    on: bool,
    env: Envelope,
    length: u16,
    length_on: bool,
    lfsr: u16,
    width7: bool,
    divisor_code: u8,
    shift: u8,
    timer: u32,
}

impl Noise {
    fn divisor(&self) -> u32 {
        match self.divisor_code {
            0 => 8,
            n => (n as u32) * 16,
        }
    }

    fn tick(&mut self) {
        if self.timer == 0 {
            self.timer = self.divisor() << self.shift;
            let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 14);
            if self.width7 {
                self.lfsr = (self.lfsr & !(1 << 6)) | (bit << 6);
            }
        } else {
            self.timer -= 1;
        }
    }

    fn sample(&self) -> i8 {
        if !self.on {
            return 0;
        }
        // Saída = bit0 invertido.
        if self.lfsr & 1 == 0 {
            self.env.volume as i8
        } else {
            -(self.env.volume as i8)
        }
    }

    fn clock_length(&mut self) {
        if self.length_on && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.on = false;
            }
        }
    }

    fn clock_envelope(&mut self) {
        self.env.clock();
    }

    fn trigger(&mut self) {
        self.on = true;
        if self.length == 0 {
            self.length = 64;
        }
        self.lfsr = 0x7FFF;
        self.timer = self.divisor() << self.shift;
        self.env.trigger();
        if self.env.initial == 0 && !self.env.add {
            self.on = false;
        }
    }

    /// Campo lógico: 0=length, 1=envelope, 2=freq/divisor, 3=trigger/length-enable.
    fn write_reg(&mut self, field: u8, val: u8) {
        match field {
            0 => self.length = 64 - (val & 0x3F) as u16, // NR41
            1 => {
                // NR42 envelope
                self.env.period = val & 0x07;
                self.env.add = val & 0x08 != 0;
                self.env.initial = val >> 4;
                if self.env.initial == 0 && !self.env.add {
                    self.on = false;
                }
            }
            2 => {
                // NR43 freq/divisor
                self.shift = val >> 4;
                self.width7 = val & 0x08 != 0;
                self.divisor_code = val & 0x07;
            }
            3 => {
                // NR44 trigger/length-enable
                self.length_on = val & 0x40 != 0;
                if val & 0x80 != 0 {
                    self.trigger();
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Liga o master, roteia todos os canais p/ L+R no volume máximo, e devolve
    /// o APU pronto pra receber escritas.
    fn apu_on() -> Apu {
        let mut a = Apu::new();
        a.write_u8(0x0400_0084, 0x80); // SOUNDCNT_X master enable
        a.write_u8(0x0400_0080, 0x77); // SOUNDCNT_L: volume L/R = 7
        a.write_u8(0x0400_0081, 0xFF); // habilita os 4 canais em L e R
        a
    }

    #[test]
    fn master_disable_blocks_channel_writes() {
        let mut a = Apu::new(); // master desligado
        a.write_u8(0x0400_0060, 0xFF); // tenta escrever no canal 1
        a.write_u8(0x0400_0084, 0x80); // liga
                                       // Após ligar, o canal 1 deve estar zerado (a escrita anterior foi ignorada).
        assert_eq!(a.ch1.sweep_shift, 0);
    }

    #[test]
    fn square_trigger_turns_channel_on() {
        let mut a = apu_on();
        a.write_u8(0x0400_0063, 0xF0); // NR12: volume 15, sem envelope
        a.write_u8(0x0400_0064, 0x00); // NR13
        a.write_u8(0x0400_0065, 0x80); // NR14: trigger
        assert!(a.ch1.on);
        assert_eq!(a.ch1.env.volume, 15);
    }

    #[test]
    fn square_dac_off_when_volume_zero_and_no_add() {
        let mut a = apu_on();
        a.write_u8(0x0400_0063, 0x00); // NR12: volume 0, add=0 → DAC off
        a.write_u8(0x0400_0065, 0x80); // trigger
        assert!(!a.ch1.on, "DAC desligado não deve ligar o canal");
    }

    #[test]
    fn square_produces_alternating_waveform() {
        let mut a = apu_on();
        a.write_u8(0x0400_0062, 0x80); // NR11: duty 50% (bits 6-7 = 10)
        a.write_u8(0x0400_0063, 0xF0); // NR12: volume 15
        a.write_u8(0x0400_0064, 0x00); // NR13: freq baixa
        a.write_u8(0x0400_0065, 0x87); // NR14: trigger + freq alta (período curto)
                                       // Avança e coleta amostras: deve haver valores positivos e negativos.
        a.tick(200_000);
        let drained = a.buffer.clone();
        assert!(drained.iter().any(|&s| s > 0));
        assert!(drained.iter().any(|&s| s < 0));
    }

    #[test]
    fn length_counter_silences_channel() {
        let mut a = apu_on();
        a.write_u8(0x0400_0062, 0x3F); // NR11: length = 64-63 = 1
        a.write_u8(0x0400_0063, 0xF0); // NR12: volume 15
        a.write_u8(0x0400_0065, 0xC0); // NR14: trigger + length enable
        assert!(a.ch1.on);
        // O 1º passo do frame sequencer (em 8192 ciclos de APU = 32768 de CPU) já
        // clocka length; com length=1 isso silencia. Roda o bastante.
        a.tick(100_000);
        assert!(!a.ch1.on, "length deveria ter silenciado o canal");
    }

    #[test]
    fn noise_lfsr_advances() {
        let mut a = apu_on();
        a.write_u8(0x0400_0079, 0xF0); // NR42: volume 15
        a.write_u8(0x0400_007C, 0x00); // NR43: shift 0, divisor 8
        a.write_u8(0x0400_007D, 0x80); // NR44: trigger
        assert!(a.ch4.on);
        let before = a.ch4.lfsr;
        a.tick(1000);
        assert_ne!(a.ch4.lfsr, before, "LFSR do ruído deveria avançar");
    }

    #[test]
    fn wave_outputs_from_ram() {
        let mut a = apu_on();
        // Preenche a wave RAM com um padrão.
        for i in 0..16 {
            a.write_u8(0x0400_0090 + i, 0xF0); // nibbles: 15, 0, 15, 0...
        }
        a.write_u8(0x0400_0070, 0x80); // NR30: DAC on
        a.write_u8(0x0400_0073, 0x20); // NR32: volume 100%
        a.write_u8(0x0400_0075, 0x80); // NR34: trigger
        assert!(a.ch3.on);
        a.tick(100_000);
        assert!(a.buffer.iter().any(|&s| s != 0), "wave deveria gerar som");
    }
}
