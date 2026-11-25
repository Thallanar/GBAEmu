//! Ícones de traço desenhados com o painter do egui, na mesma grade 24×24 dos
//! SVGs do canvas de design (sem depender de fonte de ícones nem de emoji).

use eframe::egui::{self, pos2, Color32, Pos2, Rect, Shape, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Library,
    Pause,
    Play,
    Reset,
    FastForward,
    States,
    Camera,
    Link,
    Sparkle,
    Gear,
    File,
    Close,
    Back,
}

/// Desenha `icon` centralizado em `rect` com a cor `color`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let k = side / 24.0;
    let o = rect.center() - egui::vec2(12.0 * k, 12.0 * k);
    let p = |x: f32, y: f32| -> Pos2 { pos2(o.x + x * k, o.y + y * k) };
    let stroke = Stroke::new((2.0 * k).max(1.2), color);
    let line = |pts: &[(f32, f32)]| {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| p(x, y)).collect();
        painter.add(Shape::line(pts, stroke));
    };
    let closed = |pts: &[(f32, f32)]| {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| p(x, y)).collect();
        painter.add(Shape::closed_line(pts, stroke));
    };
    let filled = |pts: &[(f32, f32)]| {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| p(x, y)).collect();
        painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
    };
    let circle = |x: f32, y: f32, r: f32| {
        painter.circle_stroke(p(x, y), r * k, stroke);
    };
    let rrect = |x: f32, y: f32, w: f32, h: f32, r: f32| {
        painter.rect_stroke(Rect::from_min_max(p(x, y), p(x + w, y + h)), r * k, stroke);
    };
    // Arco de `a0` a `a1` (graus, 0 = direita, sentido horário na tela).
    let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| {
        let n = 24;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                p(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        painter.add(Shape::line(pts, stroke));
    };

    match icon {
        Icon::Library => {
            rrect(3.0, 3.0, 7.0, 7.0, 1.5);
            rrect(14.0, 3.0, 7.0, 7.0, 1.5);
            rrect(3.0, 14.0, 7.0, 7.0, 1.5);
            rrect(14.0, 14.0, 7.0, 7.0, 1.5);
        }
        Icon::Pause => {
            line(&[(9.0, 5.0), (9.0, 19.0)]);
            line(&[(15.0, 5.0), (15.0, 19.0)]);
        }
        Icon::Play => filled(&[(7.0, 5.0), (19.0, 12.0), (7.0, 19.0)]),
        Icon::Reset => {
            arc(12.0, 12.0, 8.0, -45.0, 270.0);
            line(&[(20.0, 4.0), (20.0, 9.0), (15.0, 9.0)]);
        }
        Icon::FastForward => {
            closed(&[(4.0, 6.0), (12.0, 12.0), (4.0, 18.0)]);
            closed(&[(12.0, 6.0), (20.0, 12.0), (12.0, 18.0)]);
        }
        Icon::States => {
            closed(&[(12.0, 3.0), (21.0, 8.0), (12.0, 13.0), (3.0, 8.0)]);
            line(&[(3.0, 13.0), (12.0, 18.0), (21.0, 13.0)]);
        }
        Icon::Camera => {
            closed(&[
                (4.0, 8.0),
                (7.0, 8.0),
                (9.0, 5.0),
                (15.0, 5.0),
                (17.0, 8.0),
                (20.0, 8.0),
                (20.0, 19.0),
                (4.0, 19.0),
            ]);
            circle(12.0, 13.0, 3.5);
        }
        Icon::Link => {
            // Dois elos inclinados se cruzando.
            let tilt = |x: f32, y: f32| {
                let (s, c) = (-45f32).to_radians().sin_cos();
                let (dx, dy) = (x - 12.0, y - 12.0);
                (12.0 + dx * c - dy * s, 12.0 + dx * s + dy * c)
            };
            for cx in [8.0, 16.0] {
                let n = 32;
                let pts: Vec<(f32, f32)> = (0..n)
                    .map(|i| {
                        let a = (i as f32 / n as f32) * std::f32::consts::TAU;
                        tilt(cx + 5.5 * a.cos(), 12.0 + 3.6 * a.sin())
                    })
                    .collect();
                closed(&pts);
            }
        }
        Icon::Sparkle => closed(&[
            (12.0, 3.0),
            (14.2, 9.8),
            (21.0, 12.0),
            (14.2, 14.2),
            (12.0, 21.0),
            (9.8, 14.2),
            (3.0, 12.0),
            (9.8, 9.8),
        ]),
        Icon::Gear => {
            circle(12.0, 12.0, 3.0);
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                let (s, c) = a.sin_cos();
                line(&[
                    (12.0 + 6.5 * c, 12.0 + 6.5 * s),
                    (12.0 + 9.5 * c, 12.0 + 9.5 * s),
                ]);
            }
            circle(12.0, 12.0, 6.5);
        }
        Icon::File => {
            closed(&[
                (6.0, 3.0),
                (14.0, 3.0),
                (18.0, 7.0),
                (18.0, 21.0),
                (6.0, 21.0),
            ]);
            line(&[(14.0, 3.0), (14.0, 7.0), (18.0, 7.0)]);
        }
        Icon::Close => {
            line(&[(6.0, 6.0), (18.0, 18.0)]);
            line(&[(18.0, 6.0), (6.0, 18.0)]);
        }
        Icon::Back => line(&[(15.0, 6.0), (9.0, 12.0), (15.0, 18.0)]),
    }
}
