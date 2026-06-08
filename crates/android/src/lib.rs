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
    use jni::objects::{JByteArray, JByteBuffer, JClass, JShortArray};
    use jni::sys::{jboolean, jbyteArray, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
    use jni::JNIEnv;

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

    /// Recupera `&mut Gba` de um handle vindo de `create`.
    ///
    /// # Safety
    /// O ponteiro precisa ser válido e usado por uma única thread (o Kotlin
    /// serializa todas as chamadas na thread de emulação).
    unsafe fn gba<'a>(handle: jlong) -> Option<&'a mut Gba> {
        (handle as *mut Gba).as_mut()
    }

    /// Cria uma nova instância do emulador e devolve um ponteiro opaco.
    #[no_mangle]
    pub extern "system" fn Java_com_auroragba_NativeBridge_create(
        _env: JNIEnv,
        _class: JClass,
    ) -> jlong {
        init_logger();
        Box::into_raw(Box::new(Gba::new())) as jlong
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
            drop(Box::from_raw(handle as *mut Gba));
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
        let Some(gba) = gba(handle) else {
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
        *gba = Gba::new();
        gba.load_rom(bytes);
        // `reset` faz o direct boot (modo System, SPs, PC em 0x08000000).
        gba.reset();
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

        let fb = &gba.bus.ppu.framebuffer[..];
        let dst = match env.get_direct_buffer_address(&buffer) {
            Ok(p) if !p.is_null() => p,
            _ => {
                log::error!("renderFrame: ByteBuffer não é direto");
                return;
            }
        };
        let cap = env.get_direct_buffer_capacity(&buffer).unwrap_or(0);
        if cap < fb.len() {
            log::error!("renderFrame: buffer pequeno ({cap} < {})", fb.len());
            return;
        }
        std::ptr::copy_nonoverlapping(fb.as_ptr(), dst, fb.len());
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
}

// Stub para builds não-Android (permite cargo check no host).
#[cfg(not(target_os = "android"))]
pub fn _placeholder() {}
