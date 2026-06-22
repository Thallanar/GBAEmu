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
