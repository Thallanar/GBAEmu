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
    use auroragba_core::Gba;
    use jni::objects::{JByteArray, JByteBuffer, JClass};
    use jni::sys::{jint, jlong};
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
        // Áudio descartado neste marco (sem saída de som ainda).
        gba.bus.apu.buffer.clear();

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
}

// Stub para builds não-Android (permite cargo check no host).
#[cfg(not(target_os = "android"))]
pub fn _placeholder() {}
