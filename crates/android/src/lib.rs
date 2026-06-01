//! Bindings JNI para o frontend Android.
//!
//! Compila como `cdylib` e expõe funções `Java_*` chamáveis pelo Kotlin.

#![allow(non_snake_case)]

#[cfg(target_os = "android")]
mod android_impl {
    use jni::objects::JClass;
    use jni::sys::jlong;
    use jni::JNIEnv;

    use auroragba_core::Gba;

    /// Cria uma nova instância do emulador e devolve um ponteiro opaco.
    #[no_mangle]
    pub extern "system" fn Java_com_auroragba_NativeBridge_create(
        _env: JNIEnv,
        _class: JClass,
    ) -> jlong {
        let gba = Box::new(Gba::new());
        Box::into_raw(gba) as jlong
    }

    /// Libera a instância do emulador.
    ///
    /// # Safety
    /// `handle` deve ter sido obtido de [`Java_com_auroragba_NativeBridge_create`]
    /// e ainda não ter sido destruído.
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
}

// Stub para builds não-Android (permite cargo check no host).
#[cfg(not(target_os = "android"))]
pub fn _placeholder() {}
