# AuroraGBA — app Android

App Android (Kotlin) que roda o core do emulador via uma ponte JNI
(`crates/android`, compilada como `libauroragba_android.so`). Este primeiro
marco entrega **vídeo + controles na tela** (sem áudio/saves ainda).

## Arquitetura

- `crates/android` (Rust, `cdylib`) → `libauroragba_android.so`: expõe
  `create/destroy/loadRom/renderFrame/setButtons` (pacote `com.auroragba`,
  classe `NativeBridge`).
- `android/` (este projeto Gradle): `MainActivity` mostra o framebuffer numa
  `SurfaceView` e o `ControlsView` trata o toque. Uma thread dedicada roda o loop
  de emulação+render; todo acesso ao ponteiro nativo fica nessa thread.

## Pré-requisitos (já presentes nesta máquina)

- Android SDK em `~/Android/Sdk` (NDK 27.x, platform android-34, build-tools 34).
- Rust com os targets: `aarch64-linux-android`, `x86_64-linux-android`.
- `cargo-ndk` (`cargo install cargo-ndk`).
- JDK 17 (o Gradle 8.7/AGP 8.5 não roda em JDKs muito novos).

## Build

1. Compilar as `.so` nativas direto pra dentro do projeto (jniLibs):

   ```bash
   export ANDROID_NDK_HOME=~/Android/Sdk/ndk/27.3.13750724
   cargo ndk -t arm64-v8a -t x86_64 \
     -o android/app/src/main/jniLibs \
     build -p auroragba-android --release
   ```
2. Gerar o APK (debug):

   ```bash
   cd android
   echo "sdk.dir=$HOME/Android/Sdk" > local.properties
   JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64 ./gradlew assembleDebug
   ```

   O APK sai em `android/app/build/outputs/apk/debug/app-debug.apk`.

## Rodar no emulador

```bash
~/Android/Sdk/emulator/emulator -avd Pixel_10_Pro &
~/Android/Sdk/platform-tools/adb wait-for-device
~/Android/Sdk/platform-tools/adb install -r \
  android/app/build/outputs/apk/debug/app-debug.apk
~/Android/Sdk/platform-tools/adb shell am start -n com.auroragba/.MainActivity
```

Ao abrir, escolha uma ROM `.gba` (o botão **⏏ ROM** no topo reabre o seletor).

## Logs

```bash
~/Android/Sdk/platform-tools/adb logcat -s AuroraGBA
```
