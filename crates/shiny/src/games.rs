//! Definições por jogo. Cada submódulo declara endereços de RAM e rotinas
//! específicas. Preenchidos na Fase 6 do roadmap.

pub mod emerald {
    //! Pokémon Emerald (BPEE).
    //! RAM map: https://datacrystal.tcrf.net/wiki/Pok%C3%A9mon_Emerald/RAM_map

    pub const SAVE_BLOCK_2: u32 = 0x0202_0000;
    // TODO: endereços específicos de PID dos lendários (Rayquaza, Latios/Latias, etc.)
}

pub mod ruby_sapphire {
    //! Pokémon Ruby/Sapphire (AXVE/AXPE).
}

pub mod firered_leafgreen {
    //! Pokémon FireRed/LeafGreen (BPRE/BPGE).
}
