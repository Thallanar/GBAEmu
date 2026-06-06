//! GPIO do cartucho + RTC Seiko S-3511A.
//!
//! Alguns cartuchos (Pokémon RSE) têm um chip de GPIO ligado a um relógio de
//! tempo real. O GPIO fica mapeado na região da ROM:
//!   - 0x080000C4: Data (4 bits I/O)
//!   - 0x080000C6: Direction (4 bits; 1 = saída)
//!   - 0x080000C8: Control (bit0: 1 = registradores legíveis; 0 = lê ROM)
//!
//! Os 4 pinos do GPIO ligam no RTC:
//!   bit0 = SCK (clock) · bit1 = SIO (dado serial) · bit2 = CS (chip select).
//!
//! Protocolo serial (S-3511A): sobe CS, clocka 1 byte de **comando** (MSB first,
//! nibble alto = 0x6), depois N bytes de **dado** (LSB first) — leitura ou
//! escrita conforme o bit 0 do comando. Comandos: 0=reset, 1=control/status,
//! 2=date+time (7 bytes), 3=time (3 bytes).

use std::time::{SystemTime, UNIX_EPOCH};

/// Offsets dos registradores GPIO dentro do espaço da ROM.
pub const GPIO_DATA: u32 = 0xC4;
pub const GPIO_DIRECTION: u32 = 0xC6;
pub const GPIO_CONTROL: u32 = 0xC8;
pub const GPIO_END: u32 = 0xC9;

#[derive(Default)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
pub struct Gpio {
    data: u8,
    direction: u8,
    readable: bool,
    rtc: Rtc,
}

impl Gpio {
    /// Leitura de um registrador GPIO. `None` ⇒ o controle não está em modo
    /// legível, então o bus deve devolver o byte da ROM (open bus).
    pub fn read(&self, off: u32) -> Option<u8> {
        if !self.readable {
            return None;
        }
        Some(match off {
            GPIO_DATA => self.data & 0x0F,
            GPIO_DIRECTION => self.direction & 0x0F,
            GPIO_CONTROL => self.readable as u8,
            _ => 0, // bytes altos (0xC5/0xC7/0xC9)
        })
    }

    pub fn write(&mut self, off: u32, val: u8) {
        match off {
            GPIO_DATA => {
                let out = val & 0x0F;
                // Processa as bordas de SCK/CS e obtém o bit que o RTC dirige no
                // SIO (válido quando SIO está como entrada).
                let sio_in = self.rtc.clock(out, self.direction);
                // Pinos de saída = valor do CPU; SIO (bit1), se entrada, vem do RTC.
                let mut d = out & self.direction;
                if self.direction & 0x02 == 0 {
                    d = (d & !0x02) | ((sio_in & 1) << 1);
                }
                self.data = d & 0x0F;
            }
            GPIO_DIRECTION => self.direction = val & 0x0F,
            GPIO_CONTROL => self.readable = val & 1 != 0,
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
enum Phase {
    #[default]
    Idle,
    Command,
    Data,
}

#[derive(Default)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
struct Rtc {
    sck: bool,
    cs: bool,
    sio_out: u8,
    phase: Phase,
    bits: u8,      // bits transferidos no byte atual
    byte_buf: u8,  // byte sendo montado (escrita/comando)
    reading: bool, // comando atual é leitura?
    out_bytes: Vec<u8>,
    out_index: usize,
    expected: usize, // nº de bytes do registro atual
    status: u8,      // registrador de controle
}

impl Rtc {
    /// Processa um write nos pinos. Retorna o bit que o RTC apresenta no SIO.
    fn clock(&mut self, pins: u8, _dir: u8) -> u8 {
        let new_cs = pins & 0x04 != 0;
        let new_sck = pins & 0x01 != 0;
        let sio_cpu = (pins >> 1) & 1;

        // Borda de CS: subida inicia um comando; descida encerra.
        if !self.cs && new_cs {
            self.phase = Phase::Command;
            self.bits = 0;
            self.byte_buf = 0;
        } else if self.cs && !new_cs {
            self.phase = Phase::Idle;
        }
        self.cs = new_cs;

        // Borda de subida do SCK (com CS ativo) transfere 1 bit.
        if new_cs && !self.sck && new_sck {
            match self.phase {
                Phase::Command => {
                    // Comando: MSB first.
                    self.byte_buf = (self.byte_buf << 1) | sio_cpu;
                    self.bits += 1;
                    if self.bits == 8 {
                        self.begin_command(self.byte_buf);
                        self.bits = 0;
                        self.byte_buf = 0;
                    }
                }
                Phase::Data if self.reading => {
                    // Leitura: apresenta o bit atual (LSB first) e avança.
                    if self.out_index < self.out_bytes.len() {
                        let byte = self.out_bytes[self.out_index];
                        self.sio_out = (byte >> self.bits) & 1;
                        self.bits += 1;
                        if self.bits == 8 {
                            self.bits = 0;
                            self.out_index += 1;
                        }
                    }
                }
                Phase::Data => {
                    // Escrita: LSB first.
                    self.byte_buf |= sio_cpu << self.bits;
                    self.bits += 1;
                    if self.bits == 8 {
                        self.store_byte(self.byte_buf);
                        self.bits = 0;
                        self.byte_buf = 0;
                    }
                }
                Phase::Idle => {}
            }
        }
        self.sck = new_sck;
        self.sio_out
    }

    fn begin_command(&mut self, raw: u8) {
        // Nibble alto deve ser 0x6 (0110). Alguns jogos transferem com os bits
        // invertidos — nesse caso o nibble BAIXO é 0x6; revertemos o byte.
        let cmd = if (raw >> 4) == 0x6 {
            raw
        } else if (raw & 0x0F) == 0x6 {
            raw.reverse_bits()
        } else {
            raw
        };
        let reg = (cmd >> 1) & 0x07;
        self.reading = cmd & 1 != 0;
        self.bits = 0;
        self.byte_buf = 0;
        self.out_index = 0;

        match reg {
            0 => {
                // Reset: zera o status (sem falha de energia).
                self.status = 0;
                self.phase = Phase::Idle;
            }
            1 => {
                self.expected = 1;
                self.phase = Phase::Data;
                if self.reading {
                    // Nunca reportamos falha de energia (bit7 limpo).
                    self.out_bytes = vec![self.status & 0x7F];
                }
            }
            2 => {
                self.expected = 7;
                self.phase = Phase::Data;
                if self.reading {
                    self.out_bytes = datetime_bcd();
                }
            }
            3 => {
                self.expected = 3;
                self.phase = Phase::Data;
                if self.reading {
                    self.out_bytes = datetime_bcd()[4..7].to_vec();
                }
            }
            _ => {
                self.phase = Phase::Idle;
            }
        }
    }

    fn store_byte(&mut self, byte: u8) {
        // Só o registrador de controle precisa guardar o que foi escrito (o
        // jogo seta o modo 24h). Datetime é sempre derivado do relógio do host.
        if self.expected == 1 {
            self.status = byte;
        }
        self.out_index += 1;
        if self.out_index >= self.expected {
            self.phase = Phase::Idle;
        }
    }
}

fn bcd(v: u8) -> u8 {
    ((v / 10) << 4) | (v % 10)
}

/// Data/hora do host como 7 bytes BCD: ano, mês, dia, dia-da-semana, hora,
/// minuto, segundo (formato do S-3511A, modo 24h).
fn datetime_bcd() -> Vec<u8> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, mi, s) = (
        (tod / 3600) as u8,
        ((tod % 3600) / 60) as u8,
        (tod % 60) as u8,
    );

    // Civil-from-days (algoritmo de Howard Hinnant), epoch 1970-01-01 = dia 0.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = if month <= 2 { y + 1 } else { y };
    let yy = (year % 100) as u8;
    // Dia da semana: 1970-01-01 foi quinta (4). 0 = domingo.
    let wd = ((days % 7 + 4) % 7) as u8;

    vec![bcd(yy), bcd(month), bcd(d), wd, bcd(h), bcd(mi), bcd(s)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Envia um byte (comando, MSB first) clocando SCK; CS já ativo.
    fn send_command(g: &mut Gpio, byte: u8) {
        // direção: SCK, SIO, CS como saída (bits 0,1,2).
        g.write(GPIO_CONTROL, 1);
        g.write(GPIO_DIRECTION, 0x07);
        g.write(GPIO_DATA, 0x04); // CS=1, SCK=0
        for i in (0..8).rev() {
            let sio = (byte >> i) & 1;
            g.write(GPIO_DATA, 0x04 | (sio << 1)); // SCK=0
            g.write(GPIO_DATA, 0x05 | (sio << 1)); // SCK=1 (transfere)
        }
    }

    /// Lê um byte (LSB first) clocando SCK; SIO como entrada.
    fn read_byte(g: &mut Gpio) -> u8 {
        g.write(GPIO_DIRECTION, 0x05); // SCK, CS saída; SIO entrada
        let mut byte = 0u8;
        for i in 0..8 {
            g.write(GPIO_DATA, 0x04); // SCK=0
            g.write(GPIO_DATA, 0x05); // SCK=1 (RTC apresenta o bit)
            let bit = (g.read(GPIO_DATA).unwrap() >> 1) & 1;
            byte |= bit << i;
        }
        byte
    }

    #[test]
    fn status_read_has_no_power_failure() {
        let mut g = Gpio::default();
        // Comando: reg=1 (control), read → 0x60 | (1<<1) | 1 = 0x63.
        send_command(&mut g, 0x63);
        let status = read_byte(&mut g);
        assert_eq!(
            status & 0x80,
            0,
            "bit de falha de energia não pode estar setado"
        );
    }

    #[test]
    fn datetime_is_valid_bcd() {
        let dt = datetime_bcd();
        assert_eq!(dt.len(), 7);
        // Mês 01..12, dia 01..31 em BCD plausível.
        assert!(dt[1] >= 0x01 && dt[1] <= 0x12);
        assert!(dt[2] >= 0x01 && dt[2] <= 0x31);
        assert!(dt[4] <= 0x23); // hora 24h
    }
}
