//! Bindings JNI para o frontend Android.
//!
//! Compila como `cdylib` e expõe funções `Java_*` chamáveis pelo Kotlin
//! (pacote `com.auroragba`, classe `NativeBridge`). O Kotlin segura um ponteiro
//! opaco (`jlong`) pra instância do [`Gba`] e dirige o loop: carrega a ROM, pede
//! um frame por vez (que também copia o framebuffer pro buffer da UI) e empurra
//! o estado dos botões.

#![allow(non_snake_case)]

#[cfg(target_os = "android")]
mod android_impl {
    use auroragba_core::joypad::Button;
    use auroragba_core::{apu, Gba};
    use auroragba_shiny::games::{self, GameProfile};
    use auroragba_shiny::gfx::RomGfx;
    use auroragba_shiny::{CheckResult, Hunter};
    use jni::objects::{JByteArray, JByteBuffer, JClass, JShortArray};
    use jni::sys::{jboolean, jbyteArray, jint, jintArray, jlong, jstring, JNI_FALSE, JNI_TRUE};
    use jni::JNIEnv;

    /// Estado que vive atrás do handle: o emulador mais o driver do Shiny Hunter.
    /// O `hunter` precisa persistir entre tentativas (mantém contadores e o PRNG
    /// do host), por isso fica junto do `gba` num único bloco. `profile` é
    /// detectado pelo game code ao carregar a ROM.
    struct Emu {
        gba: Gba,
        hunter: Hunter,
        hunting: bool,
        profile: Option<&'static GameProfile>,
        target: usize,
        /// Tabelas de gráficos da ROM (sprites do alvo); localizadas no loadRom.
        rom_gfx: Option<RomGfx>,
    }

    impl Emu {
        fn new() -> Self {
            Emu {
                gba: Gba::new(),
                hunter: Hunter::new(),
                hunting: false,
                profile: None,
                target: 0,
                rom_gfx: None,
            }
        }
    }

    /// Inicializa o logger do Android uma única vez (chamado por `create`).
    fn init_logger() {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            android_logger::init_once(
                android_logger::Config::default()
                    .with_max_level(log::LevelFilter::Info)
                    .with_tag("AuroraGBA"),
            );
            log::info!("AuroraGBA native iniciado");
        });
    }

    /// Recupera `&mut Emu` de um handle vindo de `create`.
    ///
    /// # Safety
    /// O ponteiro precisa ser válido e usado por uma única thread (o Kotlin
    /// serializa todas as chamadas na thread de emulação).
    unsafe fn emu<'a>(handle: jlong) -> Option<&'a mut Emu> {
        (handle as *mut Emu).as_mut()
    }

    /// Recupera `&mut Gba` de um handle (atalho pras funções que só mexem no
    /// emulador). Mesma garantia de thread única.
    ///
    /// # Safety
    /// Ver [`emu`].
    unsafe fn gba<'a>(handle: jlong) -> Option<&'a mut Gba> {
        emu(handle).map(|e| &mut e.gba)
    }

    /// Cria uma nova instância do emulador e devolve um ponteiro opaco.
    #[no_mangle]
    pub extern "system" fn Java_com_auroragba_NativeBridge_create(
        _env: JNIEnv,
        _class: JClass,
    ) -> jlong {
        init_logger();
        Box::into_raw(Box::new(Emu::new())) as jlong
    }

    /// Libera a instância do emulador.
    ///
    /// # Safety
    /// `handle` deve ter sido obtido de `create` e ainda não ter sido destruído.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_destroy(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) {
        if handle != 0 {
            drop(Box::from_raw(handle as *mut Emu));
        }
    }

    /// Carrega uma ROM (array de bytes do Kotlin) e faz o direct boot.
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_loadRom(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        rom: JByteArray,
    ) {
        let Some(emu) = emu(handle) else {
            return;
        };
        let bytes = match env.convert_byte_array(&rom) {
            Ok(b) => b,
            Err(e) => {
                log::error!("loadRom: falha ao ler o array: {e}");
                return;
            }
        };
        log::info!("loadRom: {} bytes", bytes.len());
        emu.gba = Gba::new();
        emu.gba.load_rom(bytes);
        // `reset` faz o direct boot (modo System, SPs, PC em 0x08000000).
        emu.gba.reset();
        // Identifica o jogo pelo game code → perfil do Shiny Hunter (se houver).
        emu.profile = games::detect(&emu.gba.bus.cartridge.game_code());
        // Localiza as tabelas de sprites na ROM (pro painel do hunter).
        emu.rom_gfx = RomGfx::locate(&emu.gba.bus.cartridge.rom);
        emu.hunter = Hunter::new();
        emu.hunting = false;
        emu.target = 0;
        log::info!(
            "loadRom: shiny hunter {}",
            emu.profile.map_or("não suportado", |p| p.name),
        );
    }

    /// Copia o framebuffer (RGBA8, 240×160 = 153600 bytes) pro `ByteBuffer`
    /// direto. Não avança a emulação — usado pelo render normal (após `run_frame`)
    /// e pelo Shiny Hunter (que roda os frames por dentro de `huntStep`).
    ///
    /// # Safety
    /// `buffer` precisa ser um ByteBuffer direto com capacidade ≥ ao framebuffer.
    unsafe fn write_framebuffer(env: &JNIEnv, gba: &Gba, buffer: &JByteBuffer) {
        let fb = &gba.bus.ppu.framebuffer[..];
        let dst = match env.get_direct_buffer_address(buffer) {
            Ok(p) if !p.is_null() => p,
            _ => {
                log::error!("write_framebuffer: ByteBuffer não é direto");
                return;
            }
        };
        let cap = env.get_direct_buffer_capacity(buffer).unwrap_or(0);
        if cap < fb.len() {
            log::error!("write_framebuffer: buffer pequeno ({cap} < {})", fb.len());
            return;
        }
        std::ptr::copy_nonoverlapping(fb.as_ptr(), dst, fb.len());
    }

    /// Roda um frame e copia o framebuffer (RGBA8, 240×160 = 153600 bytes) pro
    /// `ByteBuffer` direto fornecido pelo Kotlin.
    ///
    /// # Safety
    /// `handle` precisa ser válido e `buffer` precisa ser um ByteBuffer direto
    /// com capacidade ≥ tamanho do framebuffer.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_renderFrame(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        buffer: JByteBuffer,
    ) {
        let Some(gba) = gba(handle) else {
            return;
        };
        gba.run_frame();
        // O áudio fica no buffer do APU e é consumido por `drainAudio`. Limite de
        // segurança: se ninguém estiver drenando, não deixa o buffer crescer sem
        // fim (mantém ~1 s de áudio estéreo).
        let buf = &mut gba.bus.apu.buffer;
        let cap = apu::OUTPUT_RATE as usize * 2;
        if buf.len() > cap {
            buf.drain(..buf.len() - cap);
        }

        write_framebuffer(&env, gba, &buffer);
    }

    /// Copia o framebuffer atual pro `ByteBuffer` sem avançar a emulação. O Shiny
    /// Hunter usa isso pra mostrar o frame depois de `huntStep` (que já rodou os
    /// frames por dentro).
    ///
    /// # Safety
    /// `handle` válido; `buffer` direto com capacidade suficiente.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_copyFramebuffer(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        buffer: JByteBuffer,
    ) {
        if let Some(gba) = gba(handle) {
            write_framebuffer(&env, gba, &buffer);
        }
    }

    /// Atualiza o estado dos botões. `mask` usa a ordem de bits do KEYINPUT do
    /// GBA: bit0=A, 1=B, 2=Select, 3=Start, 4=Right, 5=Left, 6=Up, 7=Down,
    /// 8=R, 9=L (1 = pressionado).
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_setButtons(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
        mask: jint,
    ) {
        let Some(gba) = gba(handle) else {
            return;
        };
        const BUTTONS: [Button; 10] = [
            Button::A,
            Button::B,
            Button::Select,
            Button::Start,
            Button::Right,
            Button::Left,
            Button::Up,
            Button::Down,
            Button::R,
            Button::L,
        ];
        let mask = mask as u32;
        for (bit, button) in BUTTONS.iter().enumerate() {
            gba.bus
                .io
                .joypad
                .set_button(*button, mask & (1 << bit) != 0);
        }
    }

    /// Copia até `out.len()` amostras (i16 intercaladas L,R a 32768 Hz) do buffer
    /// do APU pro array do Kotlin, removendo-as do buffer. Devolve quantas copiou.
    /// O Kotlin escreve isso num `AudioTrack` a 32768 Hz estéreo.
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_drainAudio(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        out: JShortArray,
    ) -> jint {
        let Some(gba) = gba(handle) else {
            return 0;
        };
        let buf = &mut gba.bus.apu.buffer;
        if buf.is_empty() {
            return 0;
        }
        let cap = env.get_array_length(&out).unwrap_or(0) as usize;
        let n = buf.len().min(cap);
        if n == 0 || env.set_short_array_region(&out, 0, &buf[..n]).is_err() {
            return 0;
        }
        buf.drain(..n);
        n as jint
    }

    // ───────────────────────── saves (.sav + estados) ───────────────────────
    //
    // O Kotlin chaveia os arquivos pelo *game code* e persiste em `filesDir`
    // (armazenamento privado do app, sem permissão). Espelha o desktop:
    // `.sav` (backup da pilha/EEPROM do cartucho) é automático; os save states
    // são acionados pelo menu. Tudo roda na thread de emulação (acesso ao
    // ponteiro), igual às demais funções desta ponte.

    /// Game code (4 chars do cabeçalho da ROM). Vazio se não houver ROM/handle.
    /// É a chave dos arquivos de save no lado Kotlin.
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_gameCode(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jstring {
        let code = gba(handle)
            .map(|g| g.bus.cartridge.game_code())
            .unwrap_or_default();
        env.new_string(code)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }

    /// O jogo carregado tem memória de save (.sav)?
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_hasSave(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jboolean {
        match gba(handle) {
            Some(g) if g.bus.cartridge.has_save() => JNI_TRUE,
            _ => JNI_FALSE,
        }
    }

    /// O backup foi alterado desde a última gravação? (Decide se vale gravar.)
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_backupDirty(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jboolean {
        match gba(handle) {
            Some(g) if g.bus.cartridge.dirty => JNI_TRUE,
            _ => JNI_FALSE,
        }
    }

    /// Marca o backup como gravado (chamar após escrever o `.sav` em disco).
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_clearBackupDirty(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) {
        if let Some(g) = gba(handle) {
            g.bus.cartridge.dirty = false;
        }
    }

    /// Devolve uma cópia dos bytes do backup (.sav) pra gravar em disco. Array
    /// vazio se não houver save/handle.
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_saveBackup(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jbyteArray {
        let bytes = gba(handle)
            .map(|g| g.bus.cartridge.backup_bytes().to_vec())
            .unwrap_or_default();
        env.byte_array_from_slice(&bytes)
            .map(|a| a.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }

    /// Carrega um `.sav` lido do disco. Devolve `true` se o tamanho bateu com o
    /// esperado pelo tipo de save detectado.
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_loadBackup(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        data: JByteArray,
    ) -> jboolean {
        let Some(gba) = gba(handle) else {
            return JNI_FALSE;
        };
        let Ok(bytes) = env.convert_byte_array(&data) else {
            return JNI_FALSE;
        };
        if gba.bus.cartridge.load_backup(&bytes) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    }

    /// Serializa o estado completo do emulador (save state) pra gravar em disco.
    /// Array vazio se não houver handle.
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_saveState(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jbyteArray {
        let blob = gba(handle).map(|g| g.save_state()).unwrap_or_default();
        env.byte_array_from_slice(&blob)
            .map(|a| a.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }

    /// Restaura um save state lido do disco por cima do jogo atual. Devolve
    /// `true` se o blob é válido e bate com a ROM carregada.
    ///
    /// # Safety
    /// `handle` precisa ser válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_loadState(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        data: JByteArray,
    ) -> jboolean {
        let Some(gba) = gba(handle) else {
            return JNI_FALSE;
        };
        let Ok(bytes) = env.convert_byte_array(&data) else {
            return JNI_FALSE;
        };
        match gba.load_state(&bytes) {
            Ok(()) => JNI_TRUE,
            Err(e) => {
                log::warn!("loadState rejeitado: {e:?}");
                JNI_FALSE
            }
        }
    }

    // ───────────────────────────── Shiny Hunter ─────────────────────────────
    //
    // O jogo precisa estar parado na frente do alvo com o save carregado. A
    // caça roda na thread GL via `huntStep` (lote de frames por chamada): o
    // Hunter amassa A, injeta a seed do RNG e, a cada encontro, checa se é shiny
    // e dá soft-reset se não for. A UI lê os contadores pelos getters.

    /// O jogo carregado é suportado pelo Shiny Hunter (perfil detectado)?
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntSupported(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jboolean {
        match emu(handle) {
            Some(e) if e.profile.is_some() => JNI_TRUE,
            _ => JNI_FALSE,
        }
    }

    /// Nome do jogo no perfil do Shiny Hunter (vazio se não suportado).
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntGameName(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jstring {
        let name = emu(handle).and_then(|e| e.profile).map_or("", |p| p.name);
        env.new_string(name)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }

    /// Quantos alvos o perfil do jogo oferece.
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntTargetCount(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jint {
        emu(handle)
            .and_then(|e| e.profile)
            .map_or(0, |p| p.targets.len() as jint)
    }

    /// Nome do alvo `i` (vazio se fora do intervalo).
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntTargetName(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        i: jint,
    ) -> jstring {
        let name = emu(handle)
            .and_then(|e| e.profile)
            .and_then(|p| p.targets.get(i as usize))
            .map_or("", |t| t.name);
        env.new_string(name)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }

    /// Inicia a caça no alvo `target`. Reinicia o Hunter (zera contadores e
    /// re-semeia o PRNG do host). Devolve `false` se o jogo não é suportado ou o
    /// alvo é inválido.
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntStart(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
        target: jint,
    ) -> jboolean {
        let Some(emu) = emu(handle) else {
            return JNI_FALSE;
        };
        let Some(profile) = emu.profile else {
            return JNI_FALSE;
        };
        if target < 0 || target as usize >= profile.targets.len() {
            return JNI_FALSE;
        }
        emu.target = target as usize;
        emu.hunter = Hunter::new();
        emu.hunting = true;
        JNI_TRUE
    }

    /// Para a caça (sem resetar contadores).
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntStop(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) {
        if let Some(e) = emu(handle) {
            e.hunting = false;
        }
    }

    /// Roda um lote de `batch` frames da caça. Devolve `true` quando achou o
    /// shiny (a caça para sozinha e o controle volta pro jogador ver a batalha).
    /// O áudio gerado é descartado (caça acelerada, sem som).
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntStep(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
        batch: jint,
    ) -> jboolean {
        let Some(emu) = emu(handle) else {
            return JNI_FALSE;
        };
        if !emu.hunting {
            return JNI_FALSE;
        }
        let Some(profile) = emu.profile else {
            emu.hunting = false;
            return JNI_FALSE;
        };
        let Some(target) = profile.targets.get(emu.target) else {
            emu.hunting = false;
            return JNI_FALSE;
        };
        // `hunter` e `gba` são campos disjuntos: dá pra emprestar os dois.
        let result = emu
            .hunter
            .tick(&mut emu.gba, profile, target, batch.max(1) as u32, 60 * 60);
        // Não tocamos o áudio da caça; limpa pra não crescer o buffer.
        emu.gba.bus.apu.buffer.clear();
        if result == CheckResult::Shiny {
            emu.hunting = false;
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    }

    /// Número de tentativas (resets) da caça atual.
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntAttempts(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jlong {
        emu(handle).map_or(0, |e| e.hunter.attempts as jlong)
    }

    /// A caça está ativa?
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntIsHunting(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jboolean {
        match emu(handle) {
            Some(e) if e.hunting => JNI_TRUE,
            _ => JNI_FALSE,
        }
    }

    /// Espécie (índice interno) lida no último encontro — confirma que a caça
    /// parou no Pokémon certo.
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntLastSpecies(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jint {
        emu(handle).map_or(0, |e| e.hunter.last_species as jint)
    }

    /// Menor `shiny_value` já visto nesta caça (`0xFFFF` = nada ainda).
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntBestShinyValue(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jint {
        emu(handle).map_or(0xFFFF, |e| e.hunter.best_shiny_value as jint)
    }

    /// Decodifica o sprite de frente do alvo `target` (64×64) direto da ROM e
    /// devolve os pixels em ARGB8888 (`0xAARRGGBB`, índice 0 = transparente),
    /// prontos pra um `Bitmap` do Kotlin. Array vazio se não der (sem perfil/gfx,
    /// espécie fora da tabela). `shiny` escolhe a paleta normal ou a shiny.
    ///
    /// # Safety
    /// `handle` válido.
    #[no_mangle]
    pub unsafe extern "system" fn Java_com_auroragba_NativeBridge_huntTargetSprite(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        target: jint,
        shiny: jboolean,
    ) -> jintArray {
        let empty = || env.new_int_array(0).map(|a| a.into_raw()).unwrap_or(std::ptr::null_mut());
        let Some(emu) = emu(handle) else {
            return empty();
        };
        let Some(profile) = emu.profile else {
            return empty();
        };
        let Some(t) = profile.targets.get(target as usize) else {
            return empty();
        };
        let Some(gfx) = emu.rom_gfx else {
            return empty();
        };
        let Some(sprite) = gfx.decode_front(&emu.gba.bus.cartridge.rom, t.species, shiny != 0) else {
            return empty();
        };
        // RGBA (bytes) → ARGB (i32 por pixel) pro Bitmap.ARGB_8888 do Android.
        let px: Vec<jint> = sprite
            .rgba
            .chunks_exact(4)
            .map(|c| {
                ((c[3] as i32) << 24) | ((c[0] as i32) << 16) | ((c[1] as i32) << 8) | c[2] as i32
            })
            .collect();
        match env.new_int_array(px.len() as jint) {
            Ok(arr) if env.set_int_array_region(&arr, 0, &px).is_ok() => arr.into_raw(),
            _ => empty(),
        }
    }
}

// Stub para builds não-Android (permite cargo check no host).
#[cfg(not(target_os = "android"))]
pub fn _placeholder() {}
