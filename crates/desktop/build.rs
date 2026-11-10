//! Embute o ícone no `.exe` do Windows (o que aparece no Explorer e no atalho).
//! O ícone da janela em si vem do `with_icon` em `main.rs`; isto aqui é só o
//! recurso do executável. Roda apenas quando o **alvo** é Windows — funciona no
//! cross-compile via MinGW (`x86_64-w64-mingw32-windres`) e no MSVC do CI.

fn main() {
    println!("cargo:rerun-if-changed=assets/auroragba.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/auroragba.ico");
        res.compile()
            .expect("falha ao embutir o ícone no executável do Windows");
    }
}
