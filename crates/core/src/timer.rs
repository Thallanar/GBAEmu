//! Timers 0-3 do GBA.
//!
//! Cada timer tem:
//!   - Counter (16 bits, leitura) — incrementa conforme prescaler
//!   - Reload (16 bits, escrita só) — valor carregado quando counter overflowa
//!     OU quando o timer é (re)habilitado
//!   - Controle (16 bits): prescaler (0=1, 1=64, 2=256, 3=1024 ciclos),
//!     cascade (timer N só conta quando N-1 overflowa), IRQ enable, enable
//!
//! Endereços I/O:
//!   0x04000100 — TM0CNT_L (counter)
//!   0x04000102 — TM0CNT_H (control)
//!   0x04000104 — TM1CNT_L
//!   ...etc.

use crate::io::irq_bits;

const PRESCALER_TABLE: [u32; 4] = [1, 64, 256, 1024];

#[derive(Default, Clone, Copy)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
pub struct Timer {
    pub counter: u16,
    pub reload: u16,
    pub control: u16,
    /// Acumulador de ciclos para o prescaler.
    cycles: u32,
}

impl Timer {
    pub fn enabled(self) -> bool {
        self.control & 0x80 != 0
    }
    pub fn cascade(self) -> bool {
        self.control & 0x04 != 0
    }
    pub fn irq_enable(self) -> bool {
        self.control & 0x40 != 0
    }
    pub fn prescaler(self) -> u32 {
        PRESCALER_TABLE[(self.control as usize) & 0b11]
    }
}

#[derive(Default)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
pub struct Timers {
    pub units: [Timer; 4],
}

/// Resultado de um `tick`: IRQs a sinalizar + nº de overflows dos timers 0/1
/// (que alimentam o Direct Sound do APU).
#[derive(Default)]
pub struct TimerTick {
    pub irqs: u16,
    pub snd_overflows: [u32; 2],
}

impl Timers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Avança `cycles` ciclos em todos os timers, retornando um bitmap de
    /// IRQs disparadas (combinável com [`Io::raise`]).
    pub fn tick(&mut self, cycles: u32) -> TimerTick {
        let mut irq_pending: u16 = 0;
        let mut snd_overflows = [0u32; 2];
        // Quantas vezes o timer anterior overflowou (para cascade).
        let mut cascade_count = 0u32;

        // O índice serve para units, o bitmap de IRQ e snd_overflows ao mesmo tempo.
        #[allow(clippy::needless_range_loop)]
        for i in 0..4 {
            let t = &mut self.units[i];
            if !t.enabled() {
                cascade_count = 0;
                continue;
            }

            // Conta os overflows deste timer neste tick (pode ser >1).
            let mut count = 0u32;
            if t.cascade() && i > 0 {
                // Timer cascateado: incrementa uma vez por overflow do anterior.
                for _ in 0..cascade_count {
                    if t.counter == 0xFFFF {
                        t.counter = t.reload;
                        count += 1;
                    } else {
                        t.counter = t.counter.wrapping_add(1);
                    }
                }
            } else {
                t.cycles = t.cycles.wrapping_add(cycles);
                let step = t.prescaler();
                while t.cycles >= step {
                    t.cycles -= step;
                    if t.counter == 0xFFFF {
                        t.counter = t.reload;
                        count += 1;
                    } else {
                        t.counter = t.counter.wrapping_add(1);
                    }
                }
            }

            if count > 0 && t.irq_enable() {
                irq_pending |= match i {
                    0 => irq_bits::TIMER0,
                    1 => irq_bits::TIMER1,
                    2 => irq_bits::TIMER2,
                    _ => irq_bits::TIMER3,
                };
            }
            // Os timers 0 e 1 alimentam o Direct Sound.
            if i < 2 {
                snd_overflows[i] = count;
            }
            cascade_count = count;
        }
        TimerTick {
            irqs: irq_pending,
            snd_overflows,
        }
    }

    pub fn read_u8(&self, addr: u32) -> u8 {
        let offset = (addr - 0x0400_0100) as usize;
        let idx = offset / 4;
        let field = offset % 4;
        let t = &self.units[idx];
        match field {
            0 => t.counter as u8,
            1 => (t.counter >> 8) as u8,
            2 => t.control as u8,
            _ => (t.control >> 8) as u8,
        }
    }

    pub fn write_u16(&mut self, addr: u32, val: u16) {
        self.write_u8(addr, val as u8);
        self.write_u8(addr + 1, (val >> 8) as u8);
    }

    pub fn write_u8(&mut self, addr: u32, val: u8) {
        let offset = (addr - 0x0400_0100) as usize;
        let idx = offset / 4;
        let field = offset % 4;
        let t = &mut self.units[idx];
        match field {
            // Counter address é write-only do reload.
            0 => t.reload = (t.reload & 0xFF00) | val as u16,
            1 => t.reload = (t.reload & 0x00FF) | ((val as u16) << 8),
            2 => {
                let prev_enabled = t.enabled();
                t.control = (t.control & 0xFF00) | val as u16;
                // Transição disable→enable carrega o counter com reload.
                if !prev_enabled && t.enabled() {
                    t.counter = t.reload;
                    t.cycles = 0;
                }
            }
            _ => t.control = (t.control & 0x00FF) | ((val as u16) << 8),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_increments_with_prescaler_1() {
        let mut t = Timers::new();
        // Habilita timer 0 com prescaler=1, IRQ off, sem cascade.
        t.write_u16(0x0400_0102, 0x0080);
        let irq = t.tick(100).irqs;
        assert_eq!(irq, 0);
        assert_eq!(t.units[0].counter, 100);
    }

    #[test]
    fn timer_overflow_raises_irq() {
        let mut t = Timers::new();
        // Reload = 0xFFFE, control: enable=1, IRQ=1, prescaler=1.
        t.write_u16(0x0400_0100, 0xFFFE);
        t.write_u16(0x0400_0102, 0x00C0); // bit6=IRQ, bit7=enable
        let irq = t.tick(3).irqs;
        assert!(
            irq & irq_bits::TIMER0 != 0,
            "deve disparar IRQ TIMER0 no overflow"
        );
        // 3 ticks: 0xFFFE→0xFFFF→reload(0xFFFE)→0xFFFF.
        assert_eq!(t.units[0].counter, 0xFFFF);
    }

    #[test]
    fn timer_cascade_only_increments_on_overflow_of_previous() {
        let mut t = Timers::new();
        // Timer 0: reload=0xFFFE, prescaler=1, enable.
        t.write_u16(0x0400_0100, 0xFFFE);
        t.write_u16(0x0400_0102, 0x0080);
        // Timer 1: cascade=1, enable=1.
        t.write_u16(0x0400_0104, 0x0000);
        t.write_u16(0x0400_0106, 0x0084); // bit2=cascade, bit7=enable
        t.tick(3); // gera 1 overflow no timer 0
        assert_eq!(t.units[1].counter, 1);
    }
}
