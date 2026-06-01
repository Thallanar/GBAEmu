//! Cartridge — ROM + save backing (SRAM/Flash/EEPROM).

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
    pub save_data: Vec<u8>,
}

impl Cartridge {
    pub fn load(&mut self, rom: Vec<u8>) {
        self.save_type = detect_save_type(&rom);
        self.rom = rom;
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
}

/// Detecta o tipo de save procurando strings conhecidas na ROM.
fn detect_save_type(rom: &[u8]) -> SaveType {
    // Heurística clássica usada por mGBA/VBA.
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
