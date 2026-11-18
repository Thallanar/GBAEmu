//! Player dos vídeos de exemplo do Shiny Hunter ("❓ Como usar").
//!
//! O egui não toca vídeo, então decodificamos o H.264 nós mesmos com o
//! `openh264` (decoder da Cisco compilado do fonte — sem dependência de sistema,
//! funciona igual no Linux e no Windows). Os vídeos ficam embutidos no binário,
//! já em **Annex B** (`.h264`, NAL units com start code), no perfil
//! *Constrained Baseline* — o único que o openh264 decodifica. Gerados a partir
//! das gravações com o ffmpeg (ver `assets/hunter-help/README.md`).
//!
//! O player decodifica **sob demanda** (um quadro por vez, no ritmo do vídeo) e
//! sobe cada quadro numa textura do egui; ao chegar no fim, recomeça (loop).

use std::time::{Duration, Instant};

use auroragba_shiny::help::HelpVideo;
use eframe::egui;
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use openh264::nal_units;

/// Taxa dos vídeos (fixa no encode: `-vf fps=15`).
const FPS: u32 = 15;

/// Bytes do vídeo embutido.
fn video_bytes(video: HelpVideo) -> &'static [u8] {
    match video {
        HelpVideo::StartersRse => {
            include_bytes!("../../../assets/hunter-help/desktop/starters_rse.h264")
        }
        HelpVideo::StartersFrlg => {
            include_bytes!("../../../assets/hunter-help/desktop/starters_frlg.h264")
        }
        HelpVideo::Wild => include_bytes!("../../../assets/hunter-help/desktop/wild.h264"),
    }
}

pub struct VideoPlayer {
    video: HelpVideo,
    /// NAL units do vídeo, na ordem.
    nals: Vec<&'static [u8]>,
    /// Próxima NAL a alimentar no decoder.
    pos: usize,
    decoder: Decoder,
    /// Instante em que o próximo quadro deve aparecer.
    next_at: Instant,
    paused: bool,
    /// Último quadro decodificado (RGBA) e tamanho.
    rgba: Vec<u8>,
    size: [usize; 2],
    texture: Option<egui::TextureHandle>,
    /// Houve quadro novo desde o último upload pra textura.
    dirty: bool,
}

impl VideoPlayer {
    pub fn new(video: HelpVideo) -> Option<Self> {
        let decoder = Decoder::new().ok()?;
        let mut p = Self {
            video,
            nals: nal_units(video_bytes(video)).collect(),
            pos: 0,
            decoder,
            next_at: Instant::now(),
            paused: false,
            rgba: Vec::new(),
            size: [0, 0],
            texture: None,
            dirty: false,
        };
        // Já deixa o 1º quadro pronto (o modal abre com imagem, não em branco).
        p.decode_next_frame();
        Some(p)
    }

    pub fn video(&self) -> HelpVideo {
        self.video
    }

    /// Alimenta NAL units até sair um quadro. No fim do vídeo, recomeça.
    fn decode_next_frame(&mut self) {
        // Duas voltas no máximo: o fim do vídeo + o recomeço.
        for _ in 0..2 {
            while self.pos < self.nals.len() {
                let nal = self.nals[self.pos];
                self.pos += 1;
                if let Ok(Some(yuv)) = self.decoder.decode(nal) {
                    let (w, h) = yuv.dimensions();
                    self.rgba.resize(w * h * 4, 0);
                    yuv.write_rgba8(&mut self.rgba);
                    self.size = [w, h];
                    self.dirty = true;
                    return;
                }
            }
            // Fim: decoder novo (estado limpo) e volta pro início.
            match Decoder::new() {
                Ok(d) => self.decoder = d,
                Err(_) => return,
            }
            self.pos = 0;
        }
    }

    /// Avança a reprodução até `now` e desenha o quadro atual com a largura dada.
    pub fn show(&mut self, ui: &mut egui::Ui, width: f32) {
        let now = Instant::now();
        let frame = Duration::from_secs(1) / FPS;
        if self.paused {
            self.next_at = now + frame;
        } else {
            // Se a janela ficou parada (ex.: minimizada), não tenta "correr atrás"
            // decodificando um monte de quadros de uma vez.
            if now.saturating_duration_since(self.next_at) > frame * 8 {
                self.next_at = now;
            }
            while now >= self.next_at {
                self.decode_next_frame();
                self.next_at += frame;
            }
            ui.ctx().request_repaint_after(self.next_at - now);
        }

        if self.dirty && self.size[0] > 0 {
            let img = egui::ColorImage::from_rgba_unmultiplied(self.size, &self.rgba);
            match &mut self.texture {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => {
                    self.texture = Some(ui.ctx().load_texture(
                        "hunter-help-video",
                        img,
                        egui::TextureOptions::LINEAR,
                    ))
                }
            }
            self.dirty = false;
        }

        if let Some(tex) = &self.texture {
            let [w, h] = self.size;
            let height = width * h as f32 / w as f32;
            ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(width, height)));
        }
        let label = if self.paused {
            "▶ Continuar"
        } else {
            "⏸ Pausar"
        };
        ui.horizontal(|ui| {
            if ui.button(label).clicked() {
                self.paused = !self.paused;
            }
            if ui.button("⏮ Do início").clicked() {
                if let Ok(d) = Decoder::new() {
                    self.decoder = d;
                    self.pos = 0;
                    self.decode_next_frame();
                    self.next_at = Instant::now() + frame;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Os 3 vídeos embutidos decodificam com o openh264 (perfil certo) e têm o
    /// tamanho esperado do encode (640 px de largura).
    #[test]
    fn embedded_videos_decode() {
        for v in [
            HelpVideo::StartersRse,
            HelpVideo::StartersFrlg,
            HelpVideo::Wild,
        ] {
            let p = VideoPlayer::new(v).expect("decoder deve iniciar");
            assert_eq!(p.size[0], 640, "{v:?}: largura");
            assert!(p.size[1] > 0, "{v:?}: altura");
            assert!(p.rgba.iter().any(|&b| b != 0), "{v:?}: quadro vazio");
        }
    }

    /// Chegar ao fim do vídeo recomeça do início sem travar.
    #[test]
    fn loops_at_end() {
        let mut p = VideoPlayer::new(HelpVideo::Wild).unwrap();
        let total = p.nals.len();
        p.pos = total; // pula pro fim
        p.dirty = false;
        p.decode_next_frame();
        assert!(p.dirty, "deveria ter decodificado o 1º quadro de novo");
        assert!(p.pos < total);
    }
}
