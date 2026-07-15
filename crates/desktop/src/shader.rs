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

/// Seleção ativa de efeito: um dos embutidos ou o shader importado de arquivo.
/// `Copy` de propósito — vai por valor pro callback de pintura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Active {
    Builtin(ShaderKind),
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
const FS_HEADER: &str = "#version 330 core\n\
    uniform sampler2D uTex;\n\
    uniform vec2 uInputSize;\n\
    uniform vec2 uOutputSize;\n\
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
    u_input: Option<glow::UniformLocation>,
    u_output: Option<glow::UniformLocation>,
    u_frame: Option<glow::UniformLocation>,
}

/// Renderizador glow: dono da textura do framebuffer, do quad e dos programas.
pub struct ShaderRenderer {
    tex: glow::Texture,
    vao: glow::VertexArray,
    programs: Vec<(ShaderKind, Program)>,
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

            Self {
                tex,
                vao,
                programs,
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

    /// Desenha o quad com o efeito `kind`. O `glViewport` já foi setado pelo
    /// egui_glow para o retângulo do callback; só desenhamos.
    pub fn paint(
        &self,
        gl: &glow::Context,
        active: Active,
        input_size: [f32; 2],
        output_size: [f32; 2],
        frame_count: i32,
    ) {
        // Programa do efeito pedido; cai no passthrough (primeiro embutido
        // disponível) se o efeito não compilou ou o custom não foi carregado.
        let prog = match active {
            Active::Custom => self.custom.as_ref(),
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
            u_input: gl.get_uniform_location(program, "uInputSize"),
            u_output: gl.get_uniform_location(program, "uOutputSize"),
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
        let output = [vp.width_px as f32, vp.height_px as f32];
        if let Ok(r) = renderer.lock() {
            r.paint(painter.gl(), active, input, output, frame_count);
        }
    });
    egui::PaintCallback {
        rect,
        callback: std::sync::Arc::new(cb),
    }
}
