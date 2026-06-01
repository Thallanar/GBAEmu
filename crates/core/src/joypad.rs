//! Joypad — entrada dos botões (KEYINPUT / KEYCNT).
//!
//! KEYINPUT (0x04000130, read-only): bits 0-9, **ativo-baixo** (0 = pressionado).
//! KEYCNT (0x04000132): máscara de teclas (bits 0-9) + IRQ enable (bit 14) +
//! condição (bit 15: 0 = OR / qualquer tecla, 1 = AND / todas as teclas).
//!
//! Referência: GBATEK, "Keypad Input".

/// Botões do GBA, na ordem dos bits de KEYINPUT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    A = 0,
    B = 1,
    Select = 2,
    Start = 3,
    Right = 4,
    Left = 5,
    Up = 6,
    Down = 7,
    R = 8,
    L = 9,
}

pub struct Joypad {
    /// Estado das teclas (ativo-baixo): 1 = solta, 0 = pressionada.
    keyinput: u16,
    pub keycnt: u16,
}

impl Joypad {
    pub fn new() -> Self {
        // Todas soltas no reset (bits 0-9 = 1).
        Self {
            keyinput: 0x03FF,
            keycnt: 0,
        }
    }

    /// Atualiza o estado de um botão.
    pub fn set_button(&mut self, button: Button, pressed: bool) {
        let bit = 1u16 << (button as u16);
        if pressed {
            self.keyinput &= !bit; // 0 = pressionado
        } else {
            self.keyinput |= bit;
        }
    }

    pub fn keyinput(&self) -> u16 {
        self.keyinput
    }

    /// A condição de IRQ de keypad está satisfeita agora?
    pub fn irq_pending(&self) -> bool {
        if self.keycnt & (1 << 14) == 0 {
            return false; // IRQ de keypad desabilitada
        }
        let mask = self.keycnt & 0x03FF;
        if mask == 0 {
            return false;
        }
        let pressed = (!self.keyinput) & 0x03FF; // 1 = pressionada
        if self.keycnt & (1 << 15) != 0 {
            // AND lógico: todas as teclas da máscara pressionadas.
            (pressed & mask) == mask
        } else {
            // OR lógico: qualquer tecla da máscara pressionada.
            (pressed & mask) != 0
        }
    }
}

impl Default for Joypad {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_are_active_low() {
        let mut j = Joypad::new();
        assert_eq!(j.keyinput(), 0x03FF, "tudo solto no reset");
        j.set_button(Button::A, true);
        assert_eq!(j.keyinput() & 1, 0, "A pressionado → bit 0 = 0");
        j.set_button(Button::A, false);
        assert_eq!(j.keyinput() & 1, 1, "A solto → bit 0 = 1");
    }

    #[test]
    fn keypad_irq_or_condition() {
        let mut j = Joypad::new();
        j.keycnt = (1 << 14) | (1 << 0); // IRQ on, OR, máscara = A
        assert!(!j.irq_pending());
        j.set_button(Button::A, true);
        assert!(j.irq_pending());
    }

    #[test]
    fn keypad_irq_and_condition() {
        let mut j = Joypad::new();
        // IRQ on, AND, máscara = A + B.
        j.keycnt = (1 << 14) | (1 << 15) | (1 << 0) | (1 << 1);
        j.set_button(Button::A, true);
        assert!(!j.irq_pending(), "só A não basta no modo AND");
        j.set_button(Button::B, true);
        assert!(j.irq_pending(), "A e B → dispara");
    }
}
