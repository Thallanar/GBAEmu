//! Pós-processamento por shader no desktop (OpenGL via glow).
//!
//! O framebuffer do GBA (240×160 RGBA8) é desenhado num quad fullscreen através
//! de um fragment shader single-pass. Os efeitos vêm da pasta canônica
//! `assets/shaders/` (mesma fonte usada pelo Android); aqui embutimos o corpo via
//! `include_str!` e prependemos o preâmbulo do dialeto GL desktop (3.30). Ver o
//! contrato de uniforms em `assets/shaders/README.md`.

use auroragba_core::{SCREEN_HEIGHT, SCREEN_WIDTH};
use eframe::egui;
use eframe::glow::{self, HasContext};

/// Efeitos disponíveis. Cada variante mapeia para um `.frag` em `assets/shaders/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShaderKind {
    #[default]
    None,
    Scanlines,
    LcdGrid,
    Lcd3x,
    Crt,
}

/// Efeitos **multipass** (vários passes encadeados; ver `assets/shaders/*.mpass`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultipassKind {
    Blur,
    ScaleFx,
}

impl MultipassKind {
    pub const ALL: [MultipassKind; 2] = [MultipassKind::Blur, MultipassKind::ScaleFx];

    /// Rótulo amigável (UI).
    pub fn label(self) -> &'static str {
        match self {
            MultipassKind::Blur => "Blur",
            MultipassKind::ScaleFx => "ScaleFX",
        }
    }

    /// Chave estável para persistência (não colide com as de `ShaderKind`).
    pub fn key(self) -> &'static str {
        match self {
            MultipassKind::Blur => "blur",
            MultipassKind::ScaleFx => "scalefx",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        MultipassKind::ALL.into_iter().find(|k| k.key() == s)
    }

    /// Manifesto `.mpass` (fonte canônica em `assets/shaders/<key>.mpass`).
    fn manifest(self) -> &'static str {
        match self {
            MultipassKind::Blur => include_str!("../../../assets/shaders/blur.mpass"),
            MultipassKind::ScaleFx => include_str!("../../../assets/shaders/scalefx.mpass"),
        }
    }
}

/// Seleção ativa de efeito: um embutido single-pass, um multipass ou o importado.
/// `Copy` de propósito — vai por valor pro callback de pintura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Active {
    Builtin(ShaderKind),
    Multipass(MultipassKind),
    Custom,
}

impl Default for Active {
    fn default() -> Self {
        Active::Builtin(ShaderKind::None)
    }
}

impl ShaderKind {
    pub const ALL: [ShaderKind; 5] = [
        ShaderKind::None,
        ShaderKind::Scanlines,
        ShaderKind::LcdGrid,
        ShaderKind::Lcd3x,
        ShaderKind::Crt,
    ];

    /// Rótulo amigável (UI).
    pub fn label(self) -> &'static str {
        match self {
            ShaderKind::None => "Nenhum",
            ShaderKind::Scanlines => "Scanlines",
            ShaderKind::LcdGrid => "LCD grid",
            ShaderKind::Lcd3x => "LCD3x",
            ShaderKind::Crt => "CRT",
        }
    }

    /// Chave estável para persistência no storage.
    pub fn key(self) -> &'static str {
        match self {
            ShaderKind::None => "none",
            ShaderKind::Scanlines => "scanlines",
            ShaderKind::LcdGrid => "lcd-grid",
            ShaderKind::Lcd3x => "lcd3x",
            ShaderKind::Crt => "crt",
        }
    }

    pub fn from_key(s: &str) -> Self {
        ShaderKind::ALL
            .into_iter()
            .find(|k| k.key() == s)
            .unwrap_or(ShaderKind::None)
    }

    /// Corpo do efeito (fonte canônica em `assets/shaders/<key>.frag`).
    fn body(self) -> &'static str {
        match self {
            ShaderKind::None => include_str!("../../../assets/shaders/none.frag"),
            ShaderKind::Scanlines => include_str!("../../../assets/shaders/scanlines.frag"),
            ShaderKind::LcdGrid => include_str!("../../../assets/shaders/lcd-grid.frag"),
            ShaderKind::Lcd3x => include_str!("../../../assets/shaders/lcd3x.frag"),
            ShaderKind::Crt => include_str!("../../../assets/shaders/crt.frag"),
        }
    }
}

/// Preâmbulo do fragment shader no dialeto GL desktop (core 3.30). Resolve os
/// aliases/uniforms do contrato e fecha com um `main()` que chama `effect`.
// `uOrigTex`/`uPrevTex` são a extensão multipass da ABI (ver README): a textura
// original (fonte da cadeia) e a saída de um passe anterior declarado (`prev = N`).
// Ficam nas units 1 e 2; passes single-pass simplesmente as ignoram.
const FS_HEADER: &str = "#version 330 core\n\
    uniform sampler2D uTex;\n\
    uniform sampler2D uOrigTex;\n\
    uniform sampler2D uPrevTex;\n\
    uniform vec2 uInputSize;\n\
    uniform vec2 uOutputSize;\n\
    uniform vec2 uOrigSize;\n\
    uniform vec2 uPrevSize;\n\
    uniform int uFrameCount;\n\
    in vec2 vTex;\n\
    out vec4 fragColor;\n\
    #define SAMPLE texture\n";
const FS_FOOTER: &str = "\nvoid main() { fragColor = effect(vTex); }\n";

const VS_SOURCE: &str = "#version 330 core\n\
    layout(location = 0) in vec2 aPos;\n\
    layout(location = 1) in vec2 aTex;\n\
    out vec2 vTex;\n\
    void main() { vTex = aTex; gl_Position = vec4(aPos, 0.0, 1.0); }\n";

/// Programa GL compilado + locations dos uniforms do contrato.
struct Program {
    program: glow::Program,
    u_tex: Option<glow::UniformLocation>,
    u_orig: Option<glow::UniformLocation>,
    u_prev: Option<glow::UniformLocation>,
    u_input: Option<glow::UniformLocation>,
    u_output: Option<glow::UniformLocation>,
    u_orig_size: Option<glow::UniformLocation>,
    u_prev_size: Option<glow::UniformLocation>,
    u_frame: Option<glow::UniformLocation>,
}

/// Um passe compilado de um efeito multipass + como sua saída é amostrada.
struct Pass {
    program: Program,
    /// Fator inteiro da textura de saída (× tamanho da fonte). Ignorado no último.
    scale: i32,
    /// Filtro (min/mag) da textura de saída deste passe: `NEAREST` ou `LINEAR`.
    filter: i32,
    /// Textura de saída em meio-float (RGBA16F) em vez de RGBA8. Necessário pra
    /// passes que guardam dados (não cor) e não podem perder precisão (ex.: ScaleFX).
    float_fb: bool,
    /// Índice de um passe anterior cuja saída este passe lê em `uPrevTex`
    /// (`prev = N` no manifesto). `None` = `uPrevTex` fica sem bind.
    prev: Option<usize>,
}

/// Textura intermediária (destino de um passe não-final): FBO + textura + tamanho.
struct Target {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    w: i32,
    h: i32,
}

/// Efeito multipass compilado: N passes + (N-1) alvos intermediários (o último
/// passe desenha na tela). Os alvos são (re)alocados sob demanda por tamanho.
struct MultipassEffect {
    passes: Vec<Pass>,
    targets: Vec<Target>,
}

/// Espec de um passe lida do manifesto (`frag`, `scale`, `filter`, `float`).
struct PassSpec {
    frag: String,
    scale: i32,
    filter: i32,
    float_fb: bool,
    prev: Option<usize>,
}

/// Corpo `effect(...)` de um `.frag` referenciado por um manifesto multipass.
/// Como o desktop embute tudo via `include_str!`, resolvemos o nome do arquivo
/// para o corpo embutido por um match estático.
fn multipass_frag_body(name: &str) -> Option<&'static str> {
    Some(match name {
        "blur-h.frag" => include_str!("../../../assets/shaders/blur-h.frag"),
        "blur-v.frag" => include_str!("../../../assets/shaders/blur-v.frag"),
        "scalefx-pass0.frag" => include_str!("../../../assets/shaders/scalefx-pass0.frag"),
        "scalefx-pass1.frag" => include_str!("../../../assets/shaders/scalefx-pass1.frag"),
        "scalefx-pass2.frag" => include_str!("../../../assets/shaders/scalefx-pass2.frag"),
        "scalefx-pass3.frag" => include_str!("../../../assets/shaders/scalefx-pass3.frag"),
        "scalefx-pass4.frag" => include_str!("../../../assets/shaders/scalefx-pass4.frag"),
        _ => return None,
    })
}

/// Faz o parse de um manifesto `.mpass` numa lista ordenada de passes. Formato
/// (ver `assets/shaders/README.md`): linhas `passN = <frag> ; scale = <int> ;
/// filter = nearest|linear`; `#` é comentário. Ordena por `N`.
fn parse_manifest(src: &str) -> Result<Vec<PassSpec>, String> {
    let mut passes: Vec<(usize, PassSpec)> = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let (key, val) = (key.trim(), val.trim());
        let Some(idx) = key
            .strip_prefix("pass")
            .and_then(|n| n.parse::<usize>().ok())
        else {
            continue; // ex.: `passes = N` (informativo) e outras chaves são ignoradas.
        };
        // val = "blur-h.frag ; scale = 1 ; filter = linear"
        let mut parts = val.split(';').map(str::trim);
        let frag = parts.next().unwrap_or("").to_string();
        if frag.is_empty() {
            return Err(format!("passe {idx} sem arquivo .frag"));
        }
        let (mut scale, mut filter, mut float_fb, mut prev) =
            (1, glow::NEAREST as i32, false, None);
        for p in parts {
            if let Some((k, v)) = p.split_once('=') {
                match k.trim() {
                    "scale" => scale = v.trim().parse().unwrap_or(1).max(1),
                    "filter" => {
                        filter = if v.trim() == "linear" {
                            glow::LINEAR as i32
                        } else {
                            glow::NEAREST as i32
                        }
                    }
                    "float" => float_fb = v.trim() == "true",
                    "prev" => prev = v.trim().parse::<usize>().ok(),
                    _ => {}
                }
            }
        }
        passes.push((
            idx,
            PassSpec {
                frag,
                scale,
                filter,
                float_fb,
                prev,
            },
        ));
    }
    if passes.is_empty() {
        return Err("manifesto sem passes".into());
    }
    passes.sort_by_key(|(i, _)| *i);
    Ok(passes.into_iter().map(|(_, s)| s).collect())
}

/// Renderizador glow: dono da textura do framebuffer, do quad e dos programas.
pub struct ShaderRenderer {
    tex: glow::Texture,
    vao: glow::VertexArray,
    programs: Vec<(ShaderKind, Program)>,
    /// Efeitos multipass compilados (passes + alvos intermediários).
    multipass: Vec<(MultipassKind, MultipassEffect)>,
    /// Programa do shader importado de arquivo (`None` até o usuário carregar um).
    custom: Option<Program>,
    /// Dimensões atualmente alocadas na textura. Mudam quando um filtro de
    /// upscale entra/sai (o buffer de entrada passa a ser maior que 240×160).
    tex_w: i32,
    tex_h: i32,
}

impl ShaderRenderer {
    /// Cria o renderizador. Compila todos os efeitos conhecidos; os que falharem
    /// são logados e omitidos (o `paint` cai no passthrough).
    pub fn new(gl: &glow::Context) -> Self {
        unsafe {
            // Textura do framebuffer: 240×160 RGBA8, NEAREST, sem wrap.
            let tex = gl.create_texture().expect("create_texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                SCREEN_WIDTH as i32,
                SCREEN_HEIGHT as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                None,
            );

            // Quad fullscreen (triangle strip): pos.xy + uv.xy. v=0 no topo, igual
            // ao Android, pra a linha 0 do framebuffer ficar no topo da tela.
            #[rustfmt::skip]
            let verts: [f32; 16] = [
                -1.0,  1.0, 0.0, 0.0,
                -1.0, -1.0, 0.0, 1.0,
                 1.0,  1.0, 1.0, 0.0,
                 1.0, -1.0, 1.0, 1.0,
            ];
            let vao = gl.create_vertex_array().expect("create_vertex_array");
            let vbo = gl.create_buffer().expect("create_buffer");
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let bytes = core::slice::from_raw_parts(
                verts.as_ptr() as *const u8,
                std::mem::size_of_val(&verts),
            );
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 16, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 16, 8);
            gl.bind_vertex_array(None);

            let mut programs = Vec::new();
            for kind in ShaderKind::ALL {
                match build_program(gl, kind) {
                    Ok(p) => programs.push((kind, p)),
                    Err(e) => log::error!("shader {:?} falhou: {e}", kind),
                }
            }

            let mut multipass = Vec::new();
            for kind in MultipassKind::ALL {
                match build_multipass(gl, kind) {
                    Ok(e) => multipass.push((kind, e)),
                    Err(e) => log::error!("multipass {:?} falhou: {e}", kind),
                }
            }

            Self {
                tex,
                vao,
                programs,
                multipass,
                custom: None,
                tex_w: SCREEN_WIDTH as i32,
                tex_h: SCREEN_HEIGHT as i32,
            }
        }
    }

    /// Compila um shader importado de arquivo (apenas o corpo `effect(...)`, mesmo
    /// contrato dos embutidos — ver `assets/shaders/README.md`) e o guarda como o
    /// efeito "custom". Em erro de compilação/link devolve o log e mantém o custom
    /// anterior intacto.
    pub fn load_custom(&mut self, gl: &glow::Context, body: &str) -> Result<(), String> {
        let prog = build_program_from_body(gl, body)?;
        if let Some(old) = self.custom.replace(prog) {
            unsafe { gl.delete_program(old.program) };
        }
        Ok(())
    }

    /// Sobe os pixels (RGBA8, `w`×`h`) para a textura. Se as dimensões mudaram
    /// desde o último upload (entrou/saiu um filtro de upscale), realoca a
    /// textura com `tex_image_2d`; senão faz o caminho rápido `tex_sub_image_2d`.
    pub fn upload(&mut self, gl: &glow::Context, w: i32, h: i32, pixels: &[u8]) {
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
            if w != self.tex_w || h != self.tex_h {
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    w,
                    h,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    Some(pixels),
                );
                self.tex_w = w;
                self.tex_h = h;
            } else {
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    w,
                    h,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(pixels),
                );
            }
        }
    }

    /// Desenha o efeito `active` no `viewport` (`[x, y, w, h]`, em pixels, que o
    /// egui_glow já setou pro retângulo do callback). Single-pass desenha direto;
    /// multipass renderiza os passes intermediários em FBOs e o último aqui.
    pub fn paint(
        &mut self,
        gl: &glow::Context,
        active: Active,
        input_size: [f32; 2],
        viewport: [i32; 4],
        frame_count: i32,
    ) {
        if let Active::Multipass(kind) = active {
            self.paint_multipass(gl, kind, viewport, frame_count);
            return;
        }

        let output_size = [viewport[2] as f32, viewport[3] as f32];
        // Programa do efeito pedido; cai no passthrough (primeiro embutido
        // disponível) se o efeito não compilou ou o custom não foi carregado.
        let prog = match active {
            Active::Custom => self.custom.as_ref(),
            Active::Multipass(_) => unreachable!(),
            Active::Builtin(kind) => self
                .programs
                .iter()
                .find(|(k, _)| *k == kind)
                .map(|(_, p)| p),
        };
        let prog = prog.or_else(|| self.programs.first().map(|(_, p)| p));
        let Some(prog) = prog else { return };

        unsafe {
            gl.use_program(Some(prog.program));
            gl.uniform_1_i32(prog.u_tex.as_ref(), 0);
            gl.uniform_2_f32(prog.u_input.as_ref(), input_size[0], input_size[1]);
            gl.uniform_2_f32(prog.u_output.as_ref(), output_size[0], output_size[1]);
            gl.uniform_1_i32(prog.u_frame.as_ref(), frame_count);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_vertex_array(None);
        }
    }

    /// Renderiza um efeito multipass: passes 0..n-1 vão pra FBOs intermediários
    /// (ping-pong), o passe final desenha no `viewport`/FBO que o egui setou.
    /// Salva e restaura o binding de FBO, o viewport e o scissor do egui.
    fn paint_multipass(
        &mut self,
        gl: &glow::Context,
        kind: MultipassKind,
        viewport: [i32; 4],
        frame_count: i32,
    ) {
        let (tex, vao) = (self.tex, self.vao);
        // Fonte = a textura já carregada por `upload` (240×160, ou maior com HQx/xBRZ).
        let (src_w, src_h) = (self.tex_w, self.tex_h);
        let Some((_, effect)) = self.multipass.iter_mut().find(|(k, _)| *k == kind) else {
            return;
        };
        let n = effect.passes.len();
        if n == 0 {
            return;
        }
        if !ensure_targets(gl, effect, src_w, src_h) {
            return; // falha ao alocar FBOs — não desenha (evita estado inconsistente).
        }

        unsafe {
            // Estado do egui a restaurar depois.
            let prev_fbo =
                std::num::NonZeroU32::new(gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING) as u32)
                    .map(glow::NativeFramebuffer);
            let scissor_was_on = gl.is_enabled(glow::SCISSOR_TEST);
            gl.disable(glow::SCISSOR_TEST);

            gl.bind_vertex_array(Some(vao));
            // A textura original (fonte da cadeia) fica fixa na unit 1 o tempo todo.
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));

            let mut in_tex = tex;
            let (mut in_w, mut in_h) = (src_w, src_h);
            for i in 0..n {
                let is_last = i == n - 1;
                let (out_w, out_h) = if is_last {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, prev_fbo);
                    gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
                    (viewport[2], viewport[3])
                } else {
                    let t = &effect.targets[i];
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.fbo));
                    gl.viewport(0, 0, t.w, t.h);
                    (t.w, t.h)
                };
                let p = &effect.passes[i];
                gl.use_program(Some(p.program.program));
                gl.uniform_1_i32(p.program.u_tex.as_ref(), 0);
                gl.uniform_1_i32(p.program.u_orig.as_ref(), 1);
                gl.uniform_1_i32(p.program.u_prev.as_ref(), 2);
                gl.uniform_2_f32(p.program.u_input.as_ref(), in_w as f32, in_h as f32);
                gl.uniform_2_f32(p.program.u_output.as_ref(), out_w as f32, out_h as f32);
                gl.uniform_2_f32(p.program.u_orig_size.as_ref(), src_w as f32, src_h as f32);
                // `uPrevTex` (unit 2): saída de um passe anterior declarado (`prev = N`).
                let (prev_tex, prev_w, prev_h) = match p.prev {
                    Some(j) if j < effect.targets.len() => {
                        let t = &effect.targets[j];
                        (t.tex, t.w, t.h)
                    }
                    // Sem `prev` (ou índice inválido): aponta pra fonte — inofensivo,
                    // já que só shaders que declaram `prev` amostram `uPrevTex`.
                    _ => (tex, src_w, src_h),
                };
                gl.uniform_2_f32(p.program.u_prev_size.as_ref(), prev_w as f32, prev_h as f32);
                gl.active_texture(glow::TEXTURE2);
                gl.bind_texture(glow::TEXTURE_2D, Some(prev_tex));
                gl.uniform_1_i32(p.program.u_frame.as_ref(), frame_count);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(in_tex));
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                if !is_last {
                    let t = &effect.targets[i];
                    in_tex = t.tex;
                    in_w = t.w;
                    in_h = t.h;
                }
            }

            gl.bind_vertex_array(None);
            // Restaura o estado do egui (o último passe já rebindou prev_fbo).
            gl.bind_framebuffer(glow::FRAMEBUFFER, prev_fbo);
            gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
            if scissor_was_on {
                gl.enable(glow::SCISSOR_TEST);
            }
        }
    }
}

/// Garante que `effect` tem N-1 alvos com o tamanho certo (`fonte × scale` por
/// passe), realocando se o tamanho da fonte mudou. Devolve `false` se algum FBO
/// não ficou completo. Chamado a cada frame (no-op quando já está do tamanho).
fn ensure_targets(
    gl: &glow::Context,
    effect: &mut MultipassEffect,
    src_w: i32,
    src_h: i32,
) -> bool {
    let need = effect.passes.len().saturating_sub(1);
    let size_of = |i: usize| {
        let s = effect.passes[i].scale;
        (src_w * s, src_h * s)
    };
    let ok = effect.targets.len() == need
        && (0..need).all(|i| {
            let (w, h) = size_of(i);
            effect.targets[i].w == w && effect.targets[i].h == h
        });
    if ok {
        return true;
    }
    unsafe {
        for t in effect.targets.drain(..) {
            gl.delete_framebuffer(t.fbo);
            gl.delete_texture(t.tex);
        }
        for i in 0..need {
            let (w, h) = size_of(i);
            match create_target(gl, w, h, effect.passes[i].filter, effect.passes[i].float_fb) {
                Some((fbo, tex)) => effect.targets.push(Target { fbo, tex, w, h }),
                None => {
                    log::error!("multipass: FBO {i} incompleto ({w}x{h})");
                    return false;
                }
            }
        }
    }
    true
}

/// Cria uma textura `w`×`h` (RGBA8 ou, se `float_fb`, RGBA16F) com o `filter`
/// dado + um FBO com ela anexada. `None` se o FBO não ficou `FRAMEBUFFER_COMPLETE`.
unsafe fn create_target(
    gl: &glow::Context,
    w: i32,
    h: i32,
    filter: i32,
    float_fb: bool,
) -> Option<(glow::Framebuffer, glow::Texture)> {
    let tex = gl.create_texture().ok()?;
    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, filter);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter);
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_WRAP_S,
        glow::CLAMP_TO_EDGE as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_WRAP_T,
        glow::CLAMP_TO_EDGE as i32,
    );
    // Passes que guardam dados (ScaleFX) usam RGBA16F pra não perder precisão em
    // 8 bits. Meio-float é filtrável e color-renderável no GL desktop core.
    let (internal, ty) = if float_fb {
        (glow::RGBA16F as i32, glow::HALF_FLOAT)
    } else {
        (glow::RGBA as i32, glow::UNSIGNED_BYTE)
    };
    gl.tex_image_2d(glow::TEXTURE_2D, 0, internal, w, h, 0, glow::RGBA, ty, None);
    let fbo = gl.create_framebuffer().ok()?;
    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
    gl.framebuffer_texture_2d(
        glow::FRAMEBUFFER,
        glow::COLOR_ATTACHMENT0,
        glow::TEXTURE_2D,
        Some(tex),
        0,
    );
    let complete = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
    gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    if complete {
        Some((fbo, tex))
    } else {
        gl.delete_framebuffer(fbo);
        gl.delete_texture(tex);
        None
    }
}

/// Compila todos os passes de um efeito multipass (reusa `build_program_from_body`).
fn build_multipass(gl: &glow::Context, kind: MultipassKind) -> Result<MultipassEffect, String> {
    let specs = parse_manifest(kind.manifest())?;
    let mut passes = Vec::with_capacity(specs.len());
    for s in specs {
        let body = multipass_frag_body(&s.frag)
            .ok_or_else(|| format!("frag desconhecido no manifesto: {}", s.frag))?;
        let program = build_program_from_body(gl, body)?;
        passes.push(Pass {
            program,
            scale: s.scale,
            filter: s.filter,
            float_fb: s.float_fb,
            prev: s.prev,
        });
    }
    Ok(MultipassEffect {
        passes,
        targets: Vec::new(),
    })
}

/// Compila VS+FS de um embutido e captura as locations dos uniforms.
fn build_program(gl: &glow::Context, kind: ShaderKind) -> Result<Program, String> {
    build_program_from_body(gl, kind.body())
}

/// Compila VS+FS a partir do corpo `effect(...)` (embutido ou importado),
/// prependendo o preâmbulo do contrato, e captura as locations dos uniforms.
fn build_program_from_body(gl: &glow::Context, body: &str) -> Result<Program, String> {
    let fs_src = format!("{FS_HEADER}{body}{FS_FOOTER}");
    unsafe {
        let vs = compile(gl, glow::VERTEX_SHADER, VS_SOURCE)?;
        let fs = compile(gl, glow::FRAGMENT_SHADER, &fs_src)?;
        let program = gl.create_program().map_err(|e| e.to_string())?;
        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);
        gl.link_program(program);
        let ok = gl.get_program_link_status(program);
        // Os shaders já podem ser deletados após o link.
        gl.detach_shader(program, vs);
        gl.detach_shader(program, fs);
        gl.delete_shader(vs);
        gl.delete_shader(fs);
        if !ok {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(format!("link: {log}"));
        }
        Ok(Program {
            u_tex: gl.get_uniform_location(program, "uTex"),
            u_orig: gl.get_uniform_location(program, "uOrigTex"),
            u_prev: gl.get_uniform_location(program, "uPrevTex"),
            u_input: gl.get_uniform_location(program, "uInputSize"),
            u_output: gl.get_uniform_location(program, "uOutputSize"),
            u_orig_size: gl.get_uniform_location(program, "uOrigSize"),
            u_prev_size: gl.get_uniform_location(program, "uPrevSize"),
            u_frame: gl.get_uniform_location(program, "uFrameCount"),
            program,
        })
    }
}

unsafe fn compile(gl: &glow::Context, ty: u32, src: &str) -> Result<glow::Shader, String> {
    let shader = gl.create_shader(ty).map_err(|e| e.to_string())?;
    gl.shader_source(shader, src);
    gl.compile_shader(shader);
    if gl.get_shader_compile_status(shader) {
        Ok(shader)
    } else {
        let log = gl.get_shader_info_log(shader);
        gl.delete_shader(shader);
        Err(format!("compile: {log}"))
    }
}

/// Constrói o `PaintCallback` do egui que desenha o framebuffer já carregado na
/// textura através do efeito `kind`, dentro de `rect`.
pub fn callback(
    rect: egui::Rect,
    renderer: std::sync::Arc<std::sync::Mutex<ShaderRenderer>>,
    active: Active,
    input: [f32; 2],
    frame_count: i32,
) -> egui::PaintCallback {
    let cb = eframe::egui_glow::CallbackFn::new(move |info, painter| {
        let vp = info.viewport_in_pixels();
        // Viewport GL (origem embaixo-à-esquerda): `from_bottom_px` já dá o y certo.
        let viewport = [vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px];
        if let Ok(mut r) = renderer.lock() {
            r.paint(painter.gl(), active, input, viewport, frame_count);
        }
    });
    egui::PaintCallback {
        rect,
        callback: std::sync::Arc::new(cb),
    }
}
