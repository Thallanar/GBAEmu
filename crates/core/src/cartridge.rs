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
pub enum SaveType {
    #[default]
    None,
    Sram,
    Flash64K,
    Flash128K,
    Eeprom,
}

#[derive(Default)]
pub struct Cartridge {
    pub rom: Vec<u8>,
    pub save_type: SaveType,
    /// Bytes físicos do backup (o que vai pro arquivo `.sav`).
    save_data: Vec<u8>,
    /// Estado volátil da máquina de comandos do Flash (ignorado nos outros tipos).
    flash: Flash,
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
            _ => {}
        }
        c
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
