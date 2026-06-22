# AuroraGBA

Emulador de Game Boy Advance em Rust com modo **Shiny Hunter**, frontends desktop (CLI) e Android, e suporte a Cable Link.

## Convenção de versão (`0.PR.commits`)

Enquanto **nada foi lançado**, a versão demonstra o esforço de desenvolvimento e segue o formato `MAJOR.MINOR.PATCH` assim:

- **MAJOR** = `0` — fixo até o primeiro release público.
- **MINOR** = número sequencial do **PR atual** (o que está sendo aberto/trabalhado).
- **PATCH** = quantidade de **commits dentro desse PR**.

Exemplos: um PR #56 com 2 commits → `0.56.2`; um PR #57 com 9 commits → `0.57.9`.

### Como numerar um PR novo

1. O número do PR é o **maior PR existente + 1** (conta PRs já fechados/mergeados também — há gaps na sequência, isso é normal):

   ```sh
   gh pr list --state all --limit 1 --json number -q '.[0].number'   # +1 = este PR
   ```

2. O PATCH é o total de commits que **este** PR terá. Se você adicionar commits depois, **reajuste o PATCH** antes do merge para refletir o total final.
3. Sempre rode `git fetch` antes de calcular: o `main` local costuma ficar atrasado em relação a `origin/main`, o que falseia a contagem.

### Onde a versão vive (manter os três em sincronia)

- `Cargo.toml` → `[workspace.package] version` (propaga para todos os crates).
- `Cargo.lock` → rode `cargo update --workspace --offline` após mudar o `Cargo.toml`.
- `android/app/build.gradle.kts` → `versionName` igual à string; `versionCode` = número do PR (inteiro **sempre crescente**, exigência da Play Store).

## Build — SEMPRE Desktop **e** Android

> **Regra:** toda mudança deve compilar nas **duas** frentes antes de abrir/atualizar um PR.
> Desktop e Android compartilham o `crates/core`; é fácil quebrar um ao mexer no outro.
> Não dê um PR por pronto sem ter buildado os dois.

### Toolchain (versões fixas desta máquina)

- **Rust**: toolchain `stable`. O CI roda com `RUSTFLAGS="-D warnings"` — trate warning como erro.
- **JDK 17** em `/usr/lib/jvm/java-17-openjdk-amd64`. O Gradle 8.7 / AGP 8.5 **não** roda em JDKs muito novos; JDK 17 é obrigatório para o Android.
- **Android SDK** em `~/Android/Sdk`: NDK `27.3.13750724`, platform `android-34`, build-tools 34.
- **Targets Rust Android**: `aarch64-linux-android`, `x86_64-linux-android`.
- **`cargo-ndk`** instalado (`cargo install cargo-ndk --locked`).

### 1. Desktop (Rust)

O crate Android é `cdylib` e exige NDK, então o build de workspace o exclui:

```bash
cargo build --workspace --exclude auroragba-android
cargo test  --workspace --exclude auroragba-android
```

Binário do app desktop: `auroragba` (em `crates/desktop`; há também o bin `smoke`).
Deps de sistema do desktop (eframe, Linux): `libgtk-3-dev libxkbcommon-dev libwayland-dev libasound2-dev libudev-dev`.

### 2. Android (lib nativa + APK)

```bash
# (a) compilar as .so nativas direto para jniLibs
export ANDROID_NDK_HOME=~/Android/Sdk/ndk/27.3.13750724
cargo ndk -t arm64-v8a -t x86_64 \
  -o android/app/src/main/jniLibs \
  build -p auroragba-android --release

# (b) gerar o APK (debug) — JAVA_HOME aponta para o JDK 17
cd android
echo "sdk.dir=$HOME/Android/Sdk" > local.properties
JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64 ./gradlew assembleDebug
# APK em: android/app/build/outputs/apk/debug/app-debug.apk
```

### 3. Antes de abrir o PR (espelha o CI)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

> Nota: o job de CI `android` só compila a lib nativa (`cargo ndk ... build`), **não** monta o APK.
> Por isso o `./gradlew assembleDebug` precisa ser rodado **localmente** — é a única forma de pegar quebras do lado Kotlin/Gradle.
