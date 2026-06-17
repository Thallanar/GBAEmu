//! Cartridge — ROM + memória de backup (SRAM/Flash/EEPROM).
//!
//! A região 0x0E000000–0x0E00FFFF mapeia a memória de backup. Há três
//! tecnologias:
//!   - **SRAM** (32 KB): acesso direto byte-a-byte, sem protocolo.
//!   - **Flash** (64 KB ou 128 KB): exige sequências de comando (apagar, gravar,
//!     ler ID do chip, trocar de banco). É o que Pokémon Gen 3 usa.
//!   - **EEPROM**: protocolo serial via DMA (ainda não implementado).
//!
//! Os bytes "físicos" do save ficam sempre em `save_data`, então a persistência
//! em arquivo `.sav` é uniforme (basta ler/gravar esse buffer). A máquina de
//! estados do Flash guarda só o estado volátil (banco atual, fase de comando).

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
pub enum SaveType {
    #[default]
    None,
    Sram,
    Flash64K,
    Flash128K,
    Eeprom,
}

#[derive(Default)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
pub struct Cartridge {
    // A ROM não vai no save state (é grande e imutável; restaurada da instância
    // viva no `load_state`).
    #[cfg_attr(feature = "save-states", serde(skip))]
    pub rom: Vec<u8>,
    pub save_type: SaveType,
    /// Bytes físicos do backup (o que vai pro arquivo `.sav`).
    save_data: Vec<u8>,
    /// Estado volátil da máquina de comandos do Flash (ignorado nos outros tipos).
    flash: Flash,
    /// Estado da máquina serial da EEPROM (ignorado nos outros tipos).
    eeprom: Eeprom,
    /// GPIO + RTC do cartucho (presente em RSE; inofensivo nos demais).
    pub gpio: crate::rtc::Gpio,
    /// Marcado a cada escrita; o frontend usa pra saber quando salvar em disco.
    pub dirty: bool,
}

impl Cartridge {
    pub fn load(&mut self, rom: Vec<u8>) {
        self.save_type = detect_save_type(&rom);
        self.rom = rom;
        self.flash = Flash::default();
        self.eeprom = Eeprom::default();
        self.dirty = false;

        // Aloca o backup do tamanho certo, em estado "apagado" (0xFF, como flash
        // virgem). SRAM e Flash são todos potências de 2, o que facilita o mask.
        self.save_data = match self.save_type {
            SaveType::None => Vec::new(),
            SaveType::Sram => vec![0xFF; 0x8000], // 32 KB
            SaveType::Flash64K => {
                self.flash.configure(false, FLASH_ID_64K);
                vec![0xFF; 0x1_0000] // 64 KB
            }
            SaveType::Flash128K => {
                self.flash.configure(true, FLASH_ID_128K);
                vec![0xFF; 0x2_0000] // 128 KB (2 bancos de 64 KB)
            }
            SaveType::Eeprom => vec![0xFF; 0x2000], // 8 KB (stub)
        };
    }

    /// Título do jogo (header, offset 0xA0, 12 bytes).
    pub fn title(&self) -> String {
        if self.rom.len() < 0xAC {
            return String::new();
        }
        let bytes = &self.rom[0xA0..0xAC];
        String::from_utf8_lossy(bytes)
            .trim_end_matches('\0')
            .to_string()
    }

    /// Game code de 4 letras (header, offset 0xAC). É o identificador usado pra
    /// casar a ROM com um perfil de jogo (ex.: "BPEE" = Pokémon Emerald).
    pub fn game_code(&self) -> String {
        if self.rom.len() < 0xB0 {
            return String::new();
        }
        String::from_utf8_lossy(&self.rom[0xAC..0xB0])
            .trim_end_matches('\0')
            .to_string()
    }

    // ───────────────────── acesso à memória de save (região 0xE/0xF) ─────────

    pub fn read_save_u8(&self, addr: u32) -> u8 {
        match self.save_type {
            SaveType::Sram => self.sram_byte(addr),
            SaveType::Flash64K | SaveType::Flash128K => self.flash.read(addr, &self.save_data),
            // EEPROM não vive aqui (é região 0xD via DMA); None = open bus.
            _ => 0xFF,
        }
    }

    pub fn write_save_u8(&mut self, addr: u32, val: u8) {
        match self.save_type {
            SaveType::Sram => {
                let idx = self.sram_index(addr);
                self.save_data[idx] = val;
                self.dirty = true;
            }
            SaveType::Flash64K | SaveType::Flash128K => {
                self.flash
                    .write(addr, val, &mut self.save_data, &mut self.dirty);
            }
            _ => {}
        }
    }

    fn sram_index(&self, addr: u32) -> usize {
        (addr as usize) & (self.save_data.len() - 1)
    }

    fn sram_byte(&self, addr: u32) -> u8 {
        self.save_data[self.sram_index(addr)]
    }

    // ───────────────────── persistência (.sav) ──────────────────────────────

    /// Bytes do backup, para gravar em arquivo. Vazio se o jogo não salva.
    pub fn backup_bytes(&self) -> &[u8] {
        &self.save_data
    }

    /// Carrega um save de arquivo. Só aceita se o tamanho bater com o esperado
    /// pelo tipo detectado (evita carregar lixo de tamanho errado).
    pub fn load_backup(&mut self, bytes: &[u8]) -> bool {
        if !self.save_data.is_empty() && bytes.len() == self.save_data.len() {
            self.save_data.copy_from_slice(bytes);
            self.dirty = false;
            true
        } else {
            false
        }
    }

    /// O jogo tem memória de save?
    pub fn has_save(&self) -> bool {
        self.save_type != SaveType::None
    }

    // ───────────────────── EEPROM (região 0x0D, serial via DMA) ──────────────

    /// O save deste jogo é EEPROM? (O bus roteia a região 0x0D pra cá.)
    pub fn is_eeprom(&self) -> bool {
        self.save_type == SaveType::Eeprom
    }

    /// Recebe 1 bit (cada escrita de halfword na região 0x0D = 1 bit; só o bit 0
    /// importa). Acumula o comando até o jogo começar a ler.
    pub fn eeprom_write_bit(&mut self, bit: u8) {
        let e = &mut self.eeprom;
        // Uma escrita logo após uma leitura inicia um novo comando.
        if e.reading {
            *e = Eeprom {
                addr_bits: e.addr_bits, // o tamanho detectado persiste
                ..Eeprom::default()
            };
        }
        if e.buffer_len < 81 {
            e.buffer = (e.buffer << 1) | (bit & 1) as u128;
            e.buffer_len += 1;
        }
    }

    /// Devolve 1 bit lido (cada leitura de halfword na região 0x0D). Na primeira
    /// leitura após um comando, decodifica o que foi escrito.
    pub fn eeprom_read_bit(&mut self) -> u8 {
        if !self.eeprom.reading {
            self.eeprom_decode();
            self.eeprom.reading = true;
        }
        let e = &mut self.eeprom;
        if e.status_poll {
            return 1; // após escrita: "pronto"
        }
        // Sequência de leitura: 4 bits dummy (0) + 64 bits de dado (MSB first).
        let b = if e.read_pos < 4 {
            0
        } else {
            let idx = e.read_pos - 4; // 0..=63
            ((e.read_value >> (63 - idx)) & 1) as u8
        };
        if e.read_pos < 68 {
            e.read_pos += 1;
        }
        b
    }

    /// Decodifica o comando bufferizado. O tamanho do endereço (6 bits = 512 B,
    /// 14 bits = 8 KB) é deduzido do comprimento do primeiro comando.
    fn eeprom_decode(&mut self) {
        let len = self.eeprom.buffer_len;
        if len < 2 {
            self.eeprom.status_poll = true; // comando incompleto: responde "pronto"
            return;
        }
        let buf = self.eeprom.buffer;
        let cmd = (buf >> (len - 2)) & 0b11;
        self.eeprom.read_pos = 0;
        match cmd {
            // Leitura: 2 (cmd) + n (addr) + 1 (stop).
            0b11 => {
                let n = if self.eeprom.addr_bits != 0 {
                    self.eeprom.addr_bits
                } else {
                    len.saturating_sub(3)
                };
                self.eeprom.addr_bits = n;
                let addr = ((buf >> 1) & block_mask(n)) as usize;
                self.eeprom.read_value = self.eeprom_load(addr);
                self.eeprom.status_poll = false;
            }
            // Escrita: 2 (cmd) + n (addr) + 64 (dado) + 1 (stop).
            0b10 => {
                let n = if self.eeprom.addr_bits != 0 {
                    self.eeprom.addr_bits
                } else {
                    len.saturating_sub(67)
                };
                self.eeprom.addr_bits = n;
                let data = (buf >> 1) as u64; // 64 bits de dado, acima do stop
                let addr = ((buf >> 65) & block_mask(n)) as usize;
                self.eeprom_store(addr, data);
                self.dirty = true;
                self.eeprom.status_poll = true;
            }
            _ => self.eeprom.status_poll = true,
        }
    }

    /// Lê um bloco de 8 bytes (64 bits) da EEPROM como u64 big-endian.
    fn eeprom_load(&self, block: usize) -> u64 {
        let len = self.save_data.len();
        if len == 0 {
            return 0;
        }
        let base = (block * 8) & (len - 1);
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.save_data[base..base + 8]);
        u64::from_be_bytes(bytes)
    }

    fn eeprom_store(&mut self, block: usize, data: u64) {
        let len = self.save_data.len();
        if len == 0 {
            return;
        }
        let base = (block * 8) & (len - 1);
        self.save_data[base..base + 8].copy_from_slice(&data.to_be_bytes());
    }
}

/// Máscara dos `n` bits baixos (endereço de bloco da EEPROM). `n` ≤ 14.
fn block_mask(n: u8) -> u128 {
    (1u128 << n) - 1
}

/// Detecta o tipo de save procurando strings conhecidas na ROM.
fn detect_save_type(rom: &[u8]) -> SaveType {
    // Heurística clássica usada por mGBA/VBA. A ordem importa: "FLASH1M_V" tem
    // que ser testado antes de "FLASH_V".
    const MARKERS: &[(&[u8], SaveType)] = &[
        (b"EEPROM_V", SaveType::Eeprom),
        (b"SRAM_V", SaveType::Sram),
        (b"FLASH1M_V", SaveType::Flash128K),
        (b"FLASH512_V", SaveType::Flash64K),
        (b"FLASH_V", SaveType::Flash64K),
    ];

    for (needle, kind) in MARKERS {
        if rom.windows(needle.len()).any(|w| w == *needle) {
            return *kind;
        }
    }
    SaveType::None
}

// ───────────────────────────── Flash ────────────────────────────────────────

/// IDs (fabricante, dispositivo) reportados no modo "chip ID". Os jogos checam
/// esses valores no boot pra decidir o tamanho do save.
/// 64 KB: Panasonic MN63F805MNP. 128 KB: Sanyo LE26FV10N1TS.
const FLASH_ID_64K: (u8, u8) = (0x32, 0x1B);
const FLASH_ID_128K: (u8, u8) = (0x62, 0x13);

/// Máquina de comandos do Flash (GBATEK, seção "GBA Cart Backup Flash ROM").
///
/// Comandos chegam como sequências de escritas:
///   `AA`→0x5555, `55`→0x2AAA, depois o comando em 0x5555:
///     0x90 = entra no modo ID · 0xF0 = sai do modo ID
///     0x80 = prepara apagamento (seguido de outra sequência + 0x10/0x30)
///     0xA0 = grava um byte (a próxima escrita é o dado)
///     0xB0 = troca de banco (próxima escrita em 0x0000 = nº do banco) [128 KB]
#[derive(Default)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
struct Flash {
    /// Banco ativo (0 ou 1), só relevante nos 128 KB.
    bank: usize,
    /// Fase da sequência de desbloqueio: 0=ocioso, 1=viu AA, 2=viu 55.
    phase: u8,
    /// Modo de leitura de ID do chip ativo.
    id_mode: bool,
    /// Viu 0x80 (preâmbulo de apagamento) — espera 0x10 (chip) ou 0x30 (setor).
    erase_armed: bool,
    /// Viu 0xA0 — a próxima escrita grava um byte.
    write_armed: bool,
    /// Viu 0xB0 — a próxima escrita em 0x0000 troca o banco.
    bank_armed: bool,
    /// Se o chip tem dois bancos (128 KB).
    bank_switchable: bool,
    /// (fabricante, dispositivo) reportados no modo ID.
    id: (u8, u8),
}

impl Flash {
    fn configure(&mut self, bank_switchable: bool, id: (u8, u8)) {
        self.bank_switchable = bank_switchable;
        self.id = id;
    }

    fn read(&self, addr: u32, data: &[u8]) -> u8 {
        let a = (addr as usize) & 0xFFFF;
        if self.id_mode {
            match a {
                0 => return self.id.0,
                1 => return self.id.1,
                _ => {}
            }
        }
        data[self.bank * 0x1_0000 + a]
    }

    fn write(&mut self, addr: u32, val: u8, data: &mut [u8], dirty: &mut bool) {
        let a = addr & 0xFFFF;

        // Escritas de "passo único" disparadas por um comando anterior.
        if self.write_armed {
            data[self.bank * 0x1_0000 + a as usize] = val;
            self.write_armed = false;
            self.phase = 0;
            *dirty = true;
            return;
        }
        if self.bank_armed {
            if a == 0 {
                self.bank = (val & 1) as usize;
            }
            self.bank_armed = false;
            self.phase = 0;
            return;
        }

        match self.phase {
            0 if a == 0x5555 && val == 0xAA => self.phase = 1,
            1 if a == 0x2AAA && val == 0x55 => self.phase = 2,
            2 => {
                self.command(a, val, data, dirty);
                self.phase = 0;
            }
            _ => {
                // 0xF0 reseta o modo ID a qualquer momento; resto quebra a sequência.
                if val == 0xF0 {
                    self.id_mode = false;
                }
                self.phase = 0;
            }
        }
    }

    fn command(&mut self, a: u32, val: u8, data: &mut [u8], dirty: &mut bool) {
        match val {
            0x90 if a == 0x5555 => self.id_mode = true,
            0xF0 if a == 0x5555 => self.id_mode = false,
            0x80 if a == 0x5555 => self.erase_armed = true,
            0xA0 if a == 0x5555 => self.write_armed = true,
            0xB0 if a == 0x5555 && self.bank_switchable => self.bank_armed = true,
            // Apagar chip inteiro.
            0x10 if a == 0x5555 && self.erase_armed => {
                data.fill(0xFF);
                self.erase_armed = false;
                *dirty = true;
            }
            // Apagar um setor de 4 KB (o endereço da escrita é o setor).
            0x30 if self.erase_armed => {
                let sector = (a as usize) & 0xF000;
                let base = self.bank * 0x1_0000 + sector;
                for b in &mut data[base..base + 0x1000] {
                    *b = 0xFF;
                }
                self.erase_armed = false;
                *dirty = true;
            }
            _ => {}
        }
    }
}

// ───────────────────────────── EEPROM ───────────────────────────────────────

/// Máquina de estados serial da EEPROM (GBATEK, "GBA Cart Backup EEPROM").
///
/// O acesso é bit-serial via DMA na região 0x0D: cada halfword escrito empurra 1
/// bit de comando; cada halfword lido puxa 1 bit. Comandos:
///   - **Leitura**: `11` + endereço (6 ou 14 bits) + `0`; depois o jogo lê 4 bits
///     dummy + 64 bits de dado (MSB first).
///   - **Escrita**: `10` + endereço + 64 bits de dado + `0`; depois o jogo lê o
///     bit de "pronto" (sempre 1 aqui).
///
/// Cada endereço seleciona um bloco de 64 bits (8 bytes). O tamanho (512 B vs
/// 8 KB) é deduzido do comprimento do primeiro comando.
#[derive(Default)]
#[cfg_attr(feature = "save-states", derive(serde::Serialize, serde::Deserialize))]
struct Eeprom {
    /// Bits de endereço do jogo: 0 = ainda não detectado, 6 = 512 B, 14 = 8 KB.
    addr_bits: u8,
    /// Bits de comando acumulados (MSB primeiro). Máx. 2+14+64+1 = 81 bits.
    buffer: u128,
    buffer_len: u8,
    /// O jogo está na fase de leitura (puxando bits)?
    reading: bool,
    /// A leitura atual é o "pronto" pós-escrita (devolve sempre 1)?
    status_poll: bool,
    /// 64 bits de dado sendo enviados na leitura.
    read_value: u64,
    /// Posição na sequência de leitura (0..=67: 4 dummy + 64 dado).
    read_pos: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: monta um cartridge com um tipo de save forçado.
    fn cart_with(save_type: SaveType) -> Cartridge {
        // Simula o que load() faria, sem precisar de uma ROM real.
        let mut c = Cartridge {
            save_type,
            ..Default::default()
        };
        match save_type {
            SaveType::Sram => c.save_data = vec![0xFF; 0x8000],
            SaveType::Flash64K => {
                c.flash.configure(false, FLASH_ID_64K);
                c.save_data = vec![0xFF; 0x1_0000];
            }
            SaveType::Flash128K => {
                c.flash.configure(true, FLASH_ID_128K);
                c.save_data = vec![0xFF; 0x2_0000];
            }
            SaveType::Eeprom => c.save_data = vec![0xFF; 0x2000],
            _ => {}
        }
        c
    }

    // Helpers da EEPROM: empurram/puxam bits como o jogo faria via DMA.
    fn ee_write_bits(c: &mut Cartridge, value: u128, nbits: usize) {
        for i in (0..nbits).rev() {
            c.eeprom_write_bit(((value >> i) & 1) as u8);
        }
    }

    fn ee_write_block(c: &mut Cartridge, addr: u128, data: u64, addr_bits: usize) {
        ee_write_bits(c, 0b10, 2); // comando de escrita
        ee_write_bits(c, addr, addr_bits);
        ee_write_bits(c, data as u128, 64);
        c.eeprom_write_bit(0); // stop
        let _ = c.eeprom_read_bit(); // poll "pronto"
    }

    fn ee_read_block(c: &mut Cartridge, addr: u128, addr_bits: usize) -> u64 {
        ee_write_bits(c, 0b11, 2); // comando de leitura
        ee_write_bits(c, addr, addr_bits);
        c.eeprom_write_bit(0); // stop
        let mut v = 0u64;
        for k in 0..68 {
            let b = c.eeprom_read_bit();
            if k >= 4 {
                v = (v << 1) | b as u64; // ignora os 4 bits dummy
            }
        }
        v
    }

    #[test]
    fn eeprom_8k_round_trip() {
        let mut c = cart_with(SaveType::Eeprom);
        ee_write_block(&mut c, 0x2A, 0x0123_4567_89AB_CDEF, 14);
        assert_eq!(c.eeprom.addr_bits, 14, "tamanho 8 KB detectado");
        assert!(c.dirty);
        assert_eq!(ee_read_block(&mut c, 0x2A, 14), 0x0123_4567_89AB_CDEF);
        // Outro bloco continua apagado (0xFF…).
        assert_eq!(ee_read_block(&mut c, 0x2B, 14), u64::MAX);
    }

    #[test]
    fn eeprom_512b_detects_6bit_address() {
        let mut c = cart_with(SaveType::Eeprom);
        ee_write_block(&mut c, 0x05, 0xDEAD_BEEF_CAFE_F00D, 6);
        assert_eq!(c.eeprom.addr_bits, 6, "tamanho 512 B detectado");
        assert_eq!(ee_read_block(&mut c, 0x05, 6), 0xDEAD_BEEF_CAFE_F00D);
    }

    #[test]
    fn eeprom_distinct_blocks() {
        let mut c = cart_with(SaveType::Eeprom);
        ee_write_block(&mut c, 0x00, 0x1111_1111_1111_1111, 14);
        ee_write_block(&mut c, 0x10, 0x2222_2222_2222_2222, 14);
        assert_eq!(ee_read_block(&mut c, 0x00, 14), 0x1111_1111_1111_1111);
        assert_eq!(ee_read_block(&mut c, 0x10, 14), 0x2222_2222_2222_2222);
    }

    #[test]
    fn eeprom_routes_through_bus() {
        use crate::bus::Bus;
        let mut bus = Bus::new();
        bus.cartridge = cart_with(SaveType::Eeprom);
        // Escreve um bloco bit a bit via halfwords na região 0x0D (como o DMA).
        let send = |bus: &mut Bus, v: u128, n: usize| {
            for i in (0..n).rev() {
                bus.write_u16(0x0D00_0000, ((v >> i) & 1) as u16);
            }
        };
        send(&mut bus, 0b10, 2);
        send(&mut bus, 0x07, 14);
        send(&mut bus, 0xA5A5_5A5A_0F0F_F0F0u64 as u128, 64);
        bus.write_u16(0x0D00_0000, 0); // stop
        let _ = bus.read_u16(0x0D00_0000); // poll
                                           // Lê de volta via bus.
        send(&mut bus, 0b11, 2);
        send(&mut bus, 0x07, 14);
        bus.write_u16(0x0D00_0000, 0);
        let mut v = 0u64;
        for k in 0..68 {
            let b = (bus.read_u16(0x0D00_0000) & 1) as u64;
            if k >= 4 {
                v = (v << 1) | b;
            }
        }
        assert_eq!(v, 0xA5A5_5A5A_0F0F_F0F0);
    }

    /// Escreve a sequência de desbloqueio AA/55 seguida de um comando em 0x5555.
    fn unlock_cmd(c: &mut Cartridge, cmd: u8) {
        c.write_save_u8(0x5555, 0xAA);
        c.write_save_u8(0x2AAA, 0x55);
        c.write_save_u8(0x5555, cmd);
    }

    #[test]
    fn sram_reads_back_what_was_written() {
        let mut c = cart_with(SaveType::Sram);
        c.write_save_u8(0x10, 0x42);
        assert_eq!(c.read_save_u8(0x10), 0x42);
        assert!(c.dirty);
    }

    #[test]
    fn sram_mirrors_above_32k() {
        let mut c = cart_with(SaveType::Sram);
        c.write_save_u8(0x0000, 0xAB);
        // 0x8000 espelha 0x0000 (SRAM de 32 KB).
        assert_eq!(c.read_save_u8(0x8000), 0xAB);
    }

    #[test]
    fn flash_chip_id_mode() {
        let mut c = cart_with(SaveType::Flash64K);
        unlock_cmd(&mut c, 0x90); // entra no modo ID
        assert_eq!(c.read_save_u8(0), FLASH_ID_64K.0);
        assert_eq!(c.read_save_u8(1), FLASH_ID_64K.1);
        // Sai do modo ID — volta a ler os dados (0xFF, virgem).
        c.write_save_u8(0x5555, 0xF0);
        assert_eq!(c.read_save_u8(0), 0xFF);
    }

    #[test]
    fn flash_program_byte() {
        let mut c = cart_with(SaveType::Flash64K);
        unlock_cmd(&mut c, 0xA0); // arma gravação
        c.write_save_u8(0x1234, 0x7E); // próxima escrita = dado
        assert_eq!(c.read_save_u8(0x1234), 0x7E);
        assert!(c.dirty);
    }

    #[test]
    fn flash_sector_erase() {
        let mut c = cart_with(SaveType::Flash64K);
        // Grava um byte e depois apaga o setor que o contém.
        unlock_cmd(&mut c, 0xA0);
        c.write_save_u8(0x2500, 0x11);
        assert_eq!(c.read_save_u8(0x2500), 0x11);

        unlock_cmd(&mut c, 0x80); // preâmbulo de apagamento
        c.write_save_u8(0x5555, 0xAA);
        c.write_save_u8(0x2AAA, 0x55);
        c.write_save_u8(0x2500, 0x30); // apaga setor de 0x2000..0x3000
        assert_eq!(c.read_save_u8(0x2500), 0xFF);
    }

    #[test]
    fn flash_chip_erase() {
        let mut c = cart_with(SaveType::Flash64K);
        unlock_cmd(&mut c, 0xA0);
        c.write_save_u8(0x4000, 0x99);

        unlock_cmd(&mut c, 0x80);
        c.write_save_u8(0x5555, 0xAA);
        c.write_save_u8(0x2AAA, 0x55);
        c.write_save_u8(0x5555, 0x10); // apaga tudo
        assert_eq!(c.read_save_u8(0x4000), 0xFF);
    }

    #[test]
    fn flash_128k_bank_switch() {
        let mut c = cart_with(SaveType::Flash128K);
        // Grava no banco 0.
        unlock_cmd(&mut c, 0xA0);
        c.write_save_u8(0x0100, 0xA1);

        // Troca pro banco 1 e grava o mesmo offset.
        unlock_cmd(&mut c, 0xB0);
        c.write_save_u8(0x0000, 0x01); // seleciona banco 1
        unlock_cmd(&mut c, 0xA0);
        c.write_save_u8(0x0100, 0xB2);
        assert_eq!(c.read_save_u8(0x0100), 0xB2);

        // Volta pro banco 0: valor original preservado.
        unlock_cmd(&mut c, 0xB0);
        c.write_save_u8(0x0000, 0x00);
        assert_eq!(c.read_save_u8(0x0100), 0xA1);
    }

    #[test]
    fn flash_128k_reports_128k_id() {
        let mut c = cart_with(SaveType::Flash128K);
        unlock_cmd(&mut c, 0x90);
        assert_eq!(c.read_save_u8(0), FLASH_ID_128K.0);
        assert_eq!(c.read_save_u8(1), FLASH_ID_128K.1);
    }

    #[test]
    fn load_backup_rejects_wrong_size() {
        let mut c = cart_with(SaveType::Flash64K);
        assert!(!c.load_backup(&[0u8; 100])); // tamanho errado
        assert!(c.load_backup(&[0xAAu8; 0x1_0000])); // tamanho certo
        assert_eq!(c.read_save_u8(0x50), 0xAA);
    }
}
