//! AuroraGBA — core de emulação do Game Boy Advance.
//!
//! Esta crate contém toda a lógica de emulação (CPU, PPU, APU, memória)
//! e é puramente computacional — não faz I/O nem desenha na tela.
//! Os frontends (desktop, android) consomem esta crate.

pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod dma;
pub mod gba;
pub mod io;
pub mod joypad;
pub mod ppu;
pub mod rtc;
pub mod timer;

pub use gba::Gba;

/// Resolução nativa do GBA.
pub const SCREEN_WIDTH: usize = 240;
pub const SCREEN_HEIGHT: usize = 160;

/// Frequência do clock principal do GBA em Hz (~16.78 MHz).
pub const CLOCK_HZ: u32 = 16_777_216;

/// Helper de (de)serialização para `Box<[u8; N]>` — as RAMs do GBA (EWRAM, VRAM,
/// etc.) são arrays grandes em heap. O `serde` só deriva `Serialize`/`Deserialize`
/// para arrays de **até 32 elementos**, então arrays maiores precisam de um
/// módulo `with` manual, indicado por `#[serde(with = "crate::boxed_bytes")]`.
///
/// Usamos *const generics* (`const N: usize`) para escrever uma única função que
/// serve a todos os tamanhos. Serializamos como uma sequência de bytes; na volta,
/// lemos para um `Vec<u8>` e convertemos para um array em caixa do tamanho certo
/// (rejeitando comprimento incompatível, p. ex. um save state corrompido).
#[cfg(feature = "save-states")]
pub(crate) mod boxed_bytes {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, const N: usize>(arr: &[u8; N], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // `&self.field` (`&Box<[u8; N]>`) sofre deref-coercion para `&[u8; N]`.
        serde_bytes_slice(&arr[..], serializer)
    }

    fn serde_bytes_slice<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        bytes.serialize(serializer)
    }

    pub fn deserialize<'de, D, const N: usize>(deserializer: D) -> Result<Box<[u8; N]>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v: Vec<u8> = Vec::deserialize(deserializer)?;
        v.into_boxed_slice()
            .try_into()
            .map_err(|_| D::Error::custom("tamanho de buffer incompatível no save state"))
    }
}
