//! Original, resolution-independent artwork. No icon fonts or external brand assets.
use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Wave,
    Grid,
    Flask,
    Record,
    Settings,
    Search,
    Heart,
    Mic,
    Headphones,
    Power,
    Volume,
    Stop,
    Plus,
    ChevronLeft,
    ChevronUp,
    ChevronDown,
    Import,
    Export,
}

/// Paint an outline icon in the supplied bounds; all hit-testing belongs to the caller.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = Canvas::new(painter, rect, 24.);
    let width = 1.65;
    match icon {
        Icon::Wave => {
            for (x, y) in [(4., 9.), (8., 5.), (12., 2.5), (16., 6.), (20., 9.)] {
                c.line(&[(x, y), (x, 24. - y)], color, width);
            }
        }
        Icon::Grid => {
            for (x, y) in [(3., 3.), (14., 3.), (3., 14.), (14., 14.)] {
                c.rect(
                    (x, y),
                    (x + 7., y + 7.),
                    1.5,
                    Color32::TRANSPARENT,
                    color,
                    width,
                );
            }
        }
        Icon::Flask => {
            c.line(&[(8., 3.), (16., 3.)], color, width);
            c.line(
                &[
                    (9., 3.),
                    (9., 9.),
                    (4.5, 18.),
                    (5.5, 21.),
                    (18.5, 21.),
                    (19.5, 18.),
                    (15., 9.),
                    (15., 3.),
                ],
                color,
                width,
            );
            c.line(&[(7., 15.), (17., 15.)], color, width);
            c.circle((10., 18.), 0.8, color);
            c.circle((13., 12.), 0.75, color);
        }
        Icon::Record => {
            c.circle_outline((12., 12.), 9., color, width);
            c.circle((12., 12.), 4.25, color);
        }
        Icon::Settings => {
            let mut points = Vec::with_capacity(33);
            for i in 0..=32 {
                let a = i as f32 * std::f32::consts::TAU / 32.;
                let radius = if i % 4 == 0 || i % 4 == 3 { 7.4 } else { 9.3 };
                points.push((12. + a.cos() * radius, 12. + a.sin() * radius));
            }
            c.line(&points, color, width);
            c.circle_outline((12., 12.), 3.1, color, width);
        }
        Icon::Search => {
            c.circle_outline((10.3, 10.3), 6.6, color, width);
            c.line(&[(15.3, 15.3), (21., 21.)], color, width);
        }
        Icon::Heart => {
            c.curve(
                [(12., 20.), (8., 17.), (2.2, 12.7), (3.2, 8.)],
                color,
                width,
            );
            c.curve([(3.2, 8.), (4.3, 3.5), (10., 3.), (12., 7.2)], color, width);
            c.curve(
                [(12., 7.2), (14., 3.), (19.7, 3.5), (20.8, 8.)],
                color,
                width,
            );
            c.curve(
                [(20.8, 8.), (21.8, 12.7), (16., 17.), (12., 20.)],
                color,
                width,
            );
        }
        Icon::Mic => {
            c.rect(
                (9., 2.5),
                (15., 14.5),
                3.,
                Color32::TRANSPARENT,
                color,
                width,
            );
            c.curve(
                [(5.8, 10.5), (5.8, 20.4), (18.2, 20.4), (18.2, 10.5)],
                color,
                width,
            );
            c.line(&[(12., 18.), (12., 22.)], color, width);
            c.line(&[(8., 22.), (16., 22.)], color, width);
        }
        Icon::Headphones => {
            c.curve(
                [(3.3, 14.5), (2.2, 0.5), (21.8, 0.5), (20.7, 14.5)],
                color,
                width,
            );
            c.rect(
                (3., 12.),
                (7.3, 20.5),
                2.,
                Color32::TRANSPARENT,
                color,
                width,
            );
            c.rect(
                (16.7, 12.),
                (21., 20.5),
                2.,
                Color32::TRANSPARENT,
                color,
                width,
            );
        }
        Icon::Power => {
            c.arc((12., 12.5), 8., -0.86, 4.01, color, width);
            c.line(&[(12., 2.), (12., 11.5)], color, width);
        }
        Icon::Volume => {
            c.line(
                &[
                    (3., 9.),
                    (7., 9.),
                    (12., 5.),
                    (12., 19.),
                    (7., 15.),
                    (3., 15.),
                    (3., 9.),
                ],
                color,
                width,
            );
            c.arc((12., 12.), 5.6, -0.8, 0.8, color, width);
            c.arc((12., 12.), 9., -0.75, 0.75, color, width);
        }
        Icon::Stop => c.rect(
            (6., 6.),
            (18., 18.),
            1.8,
            Color32::TRANSPARENT,
            color,
            width,
        ),
        Icon::Plus => {
            c.line(&[(12., 4.5), (12., 19.5)], color, width);
            c.line(&[(4.5, 12.), (19.5, 12.)], color, width);
        }
        Icon::ChevronLeft => c.line(&[(15., 5.), (8., 12.), (15., 19.)], color, width),
        Icon::ChevronUp => c.line(&[(5., 15.), (12., 8.), (19., 15.)], color, width),
        Icon::ChevronDown => c.line(&[(5., 9.), (12., 16.), (19., 9.)], color, width),
        Icon::Import | Icon::Export => {
            c.line(
                &[(4., 15.5), (4., 21.), (20., 21.), (20., 15.5)],
                color,
                width,
            );
            if icon == Icon::Import {
                c.line(&[(12., 2.5), (12., 15.5)], color, width);
                c.line(&[(8., 11.5), (12., 15.5), (16., 11.5)], color, width);
            } else {
                c.line(&[(12., 15.5), (12., 2.5)], color, width);
                c.line(&[(8., 6.5), (12., 2.5), (16., 6.5)], color, width);
            }
        }
    }
}

/// Draw a managed square thumbnail within the same circular portrait and selection ring.
pub fn paint_custom_avatar(
    painter: &egui::Painter,
    rect: Rect,
    texture: egui::TextureId,
    selected: bool,
) {
    let unit = rect.width().min(rect.height()) / 100.;
    let center = rect.center();
    let mut mesh = egui::Mesh::with_texture(texture);
    mesh.vertices.push(egui::epaint::Vertex {
        pos: center,
        uv: egui::pos2(0.5, 0.5),
        color: Color32::WHITE,
    });
    const SEGMENTS: u32 = 64;
    for i in 0..=SEGMENTS {
        let angle = std::f32::consts::TAU * i as f32 / SEGMENTS as f32;
        let direction = Vec2::angled(angle);
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + direction * (44.8 * unit),
            uv: egui::pos2(0.5 + direction.x * 0.5, 0.5 + direction.y * 0.5),
            color: Color32::WHITE,
        });
        if i > 0 {
            mesh.indices.extend_from_slice(&[0, i, i + 1]);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    if selected {
        painter.circle_stroke(
            center,
            48. * unit,
            Stroke::new(
                4.5 * unit,
                Color32::from_rgba_unmultiplied(71, 220, 240, 35),
            ),
        );
        painter.circle_stroke(
            center,
            48. * unit,
            Stroke::new(1.8 * unit, rgb(72, 221, 241)),
        );
    }
}

/// Distinct, original circular voice portraits, designed on a 100-unit artboard.
/// Builtin IDs may be supplied with or without their `builtin.` prefix.
pub fn paint_avatar(painter: &egui::Painter, rect: Rect, id: &str, selected: bool) {
    let c = Canvas::new(painter, rect, 100.);
    let id = id.strip_prefix("builtin.").unwrap_or(id);
    let background = match id {
        "clean" => rgb(173, 235, 210),
        "radio" => rgb(255, 207, 160),
        "robot" => rgb(177, 216, 251),
        "echo" => rgb(249, 189, 215),
        "space" => rgb(212, 195, 251),
        "megaphone" => rgb(255, 225, 148),
        "deep" => rgb(192, 186, 229),
        "high" => rgb(255, 196, 177),
        "alien" => rgb(205, 239, 153),
        "chorus" => rgb(247, 208, 192),
        _ => rgb(183, 210, 229),
    };
    c.circle((50., 50.), 44.8, background);
    c.circle((36., 31.), 22., Color32::from_white_alpha(33));
    match id {
        "clean" => microphone(&c),
        "radio" => radio(&c),
        "robot" => robot(&c),
        "echo" => echo(&c),
        "space" => astronaut(&c),
        "megaphone" => megaphone(&c),
        "deep" => deep(&c),
        "high" => high(&c),
        "alien" => alien(&c),
        "chorus" => chorus(&c),
        _ => custom(&c, id),
    }
    if selected {
        c.circle_outline(
            (50., 50.),
            48.,
            Color32::from_rgba_unmultiplied(71, 220, 240, 35),
            4.5,
        );
        c.circle_outline((50., 50.), 48., rgb(72, 221, 241), 1.8);
    }
}

fn microphone(c: &Canvas<'_>) {
    let ink = rgb(27, 84, 69);
    c.ellipse((50., 79.), (21., 4.), Color32::from_black_alpha(13));
    c.rect((38., 21.), (62., 60.), 12., rgb(46, 119, 95), ink, 2.4);
    c.rect(
        (42., 24.),
        (47., 53.),
        2.5,
        rgb(214, 255, 230),
        Color32::TRANSPARENT,
        0.,
    );
    for y in [31., 38., 45.] {
        c.line(&[(52., y), (58., y)], rgb(158, 220, 185), 2.);
    }
    c.curve([(30., 48.), (30., 79.), (70., 79.), (70., 48.)], ink, 4.1);
    c.line(&[(50., 71.), (50., 81.)], ink, 4.);
    c.line(&[(39., 82.), (61., 82.)], ink, 4.);
    c.star((76., 29.), 5., rgb(247, 255, 230));
    c.circle((24., 33.), 2., rgb(87, 170, 129));
}

fn radio(c: &Canvas<'_>) {
    let ink = rgb(106, 52, 33);
    c.line(&[(64., 34.), (76., 16.)], ink, 2.7);
    c.circle((76., 16.), 2.6, rgb(255, 248, 217));
    c.rect((22., 35.), (79., 77.), 9., rgb(205, 109, 62), ink, 2.4);
    c.rect(
        (25., 38.),
        (76., 42.),
        2.,
        rgb(245, 169, 109),
        Color32::TRANSPARENT,
        0.,
    );
    c.circle((42., 56.5), 15.2, ink);
    c.circle((42., 56.5), 11.8, rgb(243, 175, 110));
    for x in [34., 39., 44., 49.] {
        let inset = if x == 34. || x == 49. { 2.5 } else { 0. };
        c.line(&[(x, 48. + inset), (x, 65. - inset)], ink, 2.);
    }
    c.rect((61., 45.), (72., 52.), 1.2, rgb(255, 235, 185), ink, 1.5);
    c.line(&[(65., 47.), (65., 50.)], rgb(227, 112, 61), 1.5);
    c.circle((66.5, 64.), 6., rgb(255, 230, 170));
    c.line(&[(66.5, 60.), (66.5, 64.)], ink, 1.7);
    c.rect((29., 77.), (36., 81.), 1., ink, Color32::TRANSPARENT, 0.);
    c.rect((65., 77.), (72., 81.), 1., ink, Color32::TRANSPARENT, 0.);
}

fn robot(c: &Canvas<'_>) {
    let ink = rgb(33, 67, 105);
    c.line(&[(50., 29.), (50., 18.)], ink, 2.8);
    c.circle((50., 16.), 4., rgb(245, 131, 119));
    c.rect((19., 43.), (27., 59.), 3., rgb(85, 131, 182), ink, 2.);
    c.rect((73., 43.), (81., 59.), 3., rgb(85, 131, 182), ink, 2.);
    c.rect((26., 29.), (74., 72.), 10., rgb(75, 136, 197), ink, 2.5);
    c.rect(
        (30., 33.),
        (70., 39.),
        3.,
        rgb(143, 191, 232),
        Color32::TRANSPARENT,
        0.,
    );
    c.rect((32., 43.), (68., 58.), 6., ink, Color32::TRANSPARENT, 0.);
    c.circle((41., 50.), 4., rgb(176, 251, 247));
    c.circle((59., 50.), 4., rgb(176, 251, 247));
    c.line(&[(40., 64.), (60., 64.)], rgb(210, 238, 254), 2.2);
    c.line(&[(47., 61.5), (47., 66.5)], ink, 1.5);
    c.line(&[(54., 61.5), (54., 66.5)], ink, 1.5);
    c.rect((43., 72.), (57., 79.), 2., ink, Color32::TRANSPARENT, 0.);
    c.line(&[(36., 80.), (64., 80.)], rgb(67, 115, 165), 3.7);
}

fn echo(c: &Canvas<'_>) {
    let ink = rgb(133, 54, 96);
    c.ellipse((43., 78.), (24., 5.), Color32::from_black_alpha(12));
    c.circle((40., 43.), 19., rgb(220, 117, 162));
    c.rect(
        (35., 57.),
        (48., 75.),
        3.,
        rgb(220, 117, 162),
        Color32::TRANSPARENT,
        0.,
    );
    c.ellipse((41., 76.), (21., 8.), rgb(192, 82, 132));
    c.circle((44., 44.), 14.5, rgb(255, 222, 224));
    c.polygon(
        &[(55., 39.), (64., 49.), (55., 52.)],
        rgb(255, 222, 224),
        Color32::TRANSPARENT,
        0.,
    );
    c.line(&[(51., 41.), (54., 41.)], ink, 1.7);
    c.line(&[(55., 53.), (59., 53.)], ink, 1.7);
    c.curve([(25., 45.), (23., 24.), (50., 20.), (55., 33.)], ink, 5.5);
    c.arc((54., 49.), 19., -0.65, 0.65, rgb(168, 68, 118), 2.7);
    c.arc((54., 49.), 28., -0.65, 0.65, rgb(195, 97, 143), 2.7);
    c.circle((23., 60.), 2.2, rgb(255, 233, 232));
}

fn astronaut(c: &Canvas<'_>) {
    let ink = rgb(74, 55, 121);
    c.star((23., 25.), 4.3, rgb(255, 248, 228));
    c.star((79., 29.), 3.5, rgb(255, 248, 228));
    c.circle((19., 61.), 2., rgb(245, 232, 255));
    c.curve(
        [(72., 51.), (84., 61.), (76., 75.), (66., 75.)],
        rgb(143, 121, 186),
        3.5,
    );
    c.rect((29., 62.), (72., 83.), 12., rgb(242, 237, 255), ink, 2.2);
    c.rect((23., 39.), (30., 55.), 3., rgb(180, 164, 215), ink, 1.8);
    c.rect((71., 39.), (78., 55.), 3., rgb(180, 164, 215), ink, 1.8);
    c.circle((50., 44.), 25., rgb(250, 247, 255));
    c.circle_outline((50., 44.), 25., ink, 2.2);
    c.ellipse((50., 44.), (19., 16.), rgb(103, 83, 162));
    c.ellipse((53., 48.), (14., 9.), rgb(78, 65, 136));
    c.curve(
        [(37., 42.), (39., 34.), (48., 31.), (55., 34.)],
        rgb(218, 211, 255),
        3.2,
    );
    c.circle((62., 43.), 2., rgb(164, 245, 235));
    c.rect(
        (43., 71.),
        (58., 79.),
        2.,
        rgb(178, 161, 219),
        Color32::TRANSPARENT,
        0.,
    );
    c.circle((47., 75.), 1.7, rgb(75, 160, 151));
    c.circle((54., 75.), 1.7, rgb(234, 143, 121));
}

fn megaphone(c: &Canvas<'_>) {
    let ink = rgb(126, 85, 25);
    c.polygon(
        &[(35., 55.), (48., 60.), (44., 79.), (35., 79.)],
        rgb(200, 131, 38),
        ink,
        2.1,
    );
    c.rect((23., 41.), (41., 60.), 5., rgb(230, 165, 54), ink, 2.1);
    c.polygon(
        &[(38., 40.), (69., 27.), (69., 69.), (38., 60.)],
        rgb(255, 247, 212),
        ink,
        2.4,
    );
    c.polygon(
        &[(40., 52.), (67., 58.), (67., 66.), (40., 58.)],
        rgb(244, 218, 147),
        Color32::TRANSPARENT,
        0.,
    );
    c.ellipse((69., 48.), (8., 21.), ink);
    c.ellipse((69., 48.), (4.5, 15.), rgb(224, 160, 54));
    c.line(&[(82., 36.), (87., 32.)], ink, 2.3);
    c.line(&[(84., 47.), (91., 47.)], ink, 2.3);
    c.line(&[(82., 59.), (87., 63.)], ink, 2.3);
    c.circle((22., 29.), 2.2, rgb(255, 253, 225));
}

fn deep(c: &Canvas<'_>) {
    let ink = rgb(68, 50, 98);
    c.ellipse((50., 79.), (24., 7.), rgb(107, 83, 140));
    c.rect(
        (43., 61.),
        (57., 78.),
        4.,
        rgb(179, 130, 119),
        Color32::TRANSPARENT,
        0.,
    );
    c.ellipse((50., 44.), (18., 23.), rgb(228, 173, 147));
    c.arc((50., 36.), 19., -3.15, -0.04, ink, 9.);
    c.ellipse((50., 60.), (17., 14.), ink);
    c.ellipse((50., 54.), (10., 7.), rgb(228, 173, 147));
    c.line(&[(43., 40.), (47., 40.)], ink, 2.1);
    c.line(&[(54., 40.), (58., 40.)], ink, 2.1);
    c.curve([(45., 54.), (48., 51.), (52., 51.), (55., 54.)], ink, 2.4);
    for (x, extent) in [(21., 5.), (26., 9.), (75., 8.), (80., 4.)] {
        c.line(
            &[(x, 52. - extent), (x, 52. + extent)],
            rgb(133, 110, 171),
            2.6,
        );
    }
}

fn high(c: &Canvas<'_>) {
    let ink = rgb(69, 99, 94);
    c.polygon(
        &[(39., 56.), (24., 72.), (43., 68.)],
        rgb(52, 143, 135),
        Color32::TRANSPARENT,
        0.,
    );
    c.ellipse((48., 58.), (17., 21.), rgb(81, 165, 149));
    c.polygon(
        &[(42., 50.), (24., 35.), (35., 63.)],
        rgb(43, 128, 124),
        Color32::TRANSPARENT,
        0.,
    );
    c.circle((57., 38.), 14., rgb(232, 245, 209));
    c.circle((61., 35.), 2.2, ink);
    c.polygon(
        &[(69., 39.), (83., 44.), (69., 46.)],
        rgb(192, 108, 64),
        Color32::TRANSPARENT,
        0.,
    );
    c.curve(
        [(47., 73.), (52., 79.), (57., 76.), (60., 68.)],
        rgb(229, 242, 210),
        3.2,
    );
    c.line(&[(72., 27.), (72., 20.), (78., 18.)], rgb(173, 85, 72), 2.);
    c.ellipse((69.5, 28.), (3.2, 2.2), rgb(173, 85, 72));
    c.star((22., 53.), 3.2, rgb(255, 248, 226));
}

fn alien(c: &Canvas<'_>) {
    let ink = rgb(41, 91, 73);
    c.ellipse((50., 77.), (21., 6.), rgb(135, 185, 100));
    c.rect(
        (44., 60.),
        (56., 77.),
        4.,
        rgb(80, 151, 110),
        Color32::TRANSPARENT,
        0.,
    );
    c.ellipse((50., 44.), (24., 27.), rgb(93, 169, 117));
    c.ellipse((48., 36.), (17., 16.), rgb(118, 192, 134));
    c.polygon(
        &[(29., 39.), (45., 45.), (43., 56.), (34., 51.)],
        ink,
        Color32::TRANSPARENT,
        0.,
    );
    c.polygon(
        &[(71., 39.), (55., 45.), (57., 56.), (66., 51.)],
        ink,
        Color32::TRANSPARENT,
        0.,
    );
    c.circle((35., 44.), 2.2, rgb(225, 249, 192));
    c.circle((64., 44.), 2.2, rgb(225, 249, 192));
    c.curve([(44., 62.), (48., 65.), (52., 65.), (56., 62.)], ink, 2.1);
    c.star((80., 25.), 3.6, rgb(251, 255, 223));
    c.circle((23., 65.), 2.1, rgb(164, 205, 114));
}

fn chorus(c: &Canvas<'_>) {
    c.ellipse((31., 67.), (16., 14.), rgb(165, 118, 168));
    c.circle((31., 43.), 13., rgb(190, 146, 190));
    c.ellipse((70., 67.), (16., 14.), rgb(77, 139, 157));
    c.circle((70., 43.), 13., rgb(107, 168, 179));
    c.ellipse((50., 74.), (19., 12.), rgb(221, 137, 107));
    c.circle((50., 50.), 16., rgb(255, 229, 187));
    let ink = rgb(116, 74, 67);
    for x in [44., 56.] {
        c.circle((x, 48.), 1.5, ink);
    }
    c.ellipse((50., 57.), (3.4, 4.1), ink);
    c.line(&[(25., 42.), (29., 42.)], rgb(104, 69, 115), 1.8);
    c.line(&[(33., 42.), (37., 42.)], rgb(104, 69, 115), 1.8);
    c.ellipse((31., 49.), (2.2, 2.8), rgb(104, 69, 115));
    c.line(&[(64., 42.), (68., 42.)], rgb(35, 99, 115), 1.8);
    c.line(&[(72., 42.), (76., 42.)], rgb(35, 99, 115), 1.8);
    c.ellipse((70., 49.), (2.2, 2.8), rgb(35, 99, 115));
}

fn custom(c: &Canvas<'_>, id: &str) {
    let hash = id.bytes().fold(0_u32, |value, byte| {
        value.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    let ink = rgb(55, 89, 117);
    c.circle((50., 50.), 27., rgb(218, 236, 244));
    for i in 0..5 {
        let height = 7. + ((hash.rotate_right(i * 5) & 15) as f32);
        let x = 31. + i as f32 * 9.5;
        c.line(&[(x, 50. - height), (x, 50. + height)], ink, 4.5);
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

struct Canvas<'a> {
    painter: &'a egui::Painter,
    origin: Pos2,
    scale: f32,
}
impl<'a> Canvas<'a> {
    fn new(painter: &'a egui::Painter, rect: Rect, units: f32) -> Self {
        let size = rect.width().min(rect.height()).max(0.);
        Self {
            painter,
            origin: rect.center() - Vec2::splat(size * 0.5),
            scale: size / units,
        }
    }
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + egui::vec2(x, y) * self.scale
    }
    fn stroke(&self, color: Color32, width: f32) -> Stroke {
        if width == 0. {
            Stroke::NONE
        } else {
            Stroke::new((width * self.scale).max(0.55), color)
        }
    }
    fn line(&self, points: &[(f32, f32)], color: Color32, width: f32) {
        self.painter.add(Shape::line(
            points.iter().map(|&(x, y)| self.p(x, y)).collect(),
            self.stroke(color, width),
        ));
    }
    fn curve(&self, points: [(f32, f32); 4], color: Color32, width: f32) {
        self.painter
            .add(egui::epaint::CubicBezierShape::from_points_stroke(
                points.map(|(x, y)| self.p(x, y)),
                false,
                Color32::TRANSPARENT,
                self.stroke(color, width),
            ));
    }
    fn circle(&self, center: (f32, f32), radius: f32, fill: Color32) {
        self.painter
            .circle_filled(self.p(center.0, center.1), radius * self.scale, fill);
    }
    fn circle_outline(&self, center: (f32, f32), radius: f32, color: Color32, width: f32) {
        self.painter.circle_stroke(
            self.p(center.0, center.1),
            radius * self.scale,
            self.stroke(color, width),
        );
    }
    fn ellipse(&self, center: (f32, f32), radius: (f32, f32), fill: Color32) {
        self.painter.add(Shape::ellipse_filled(
            self.p(center.0, center.1),
            egui::vec2(radius.0, radius.1) * self.scale,
            fill,
        ));
    }
    fn rect(
        &self,
        min: (f32, f32),
        max: (f32, f32),
        radius: f32,
        fill: Color32,
        color: Color32,
        width: f32,
    ) {
        self.painter.rect(
            Rect::from_min_max(self.p(min.0, min.1), self.p(max.0, max.1)),
            radius * self.scale,
            fill,
            self.stroke(color, width),
            egui::StrokeKind::Inside,
        );
    }
    fn polygon(&self, points: &[(f32, f32)], fill: Color32, color: Color32, width: f32) {
        self.painter.add(Shape::convex_polygon(
            points.iter().map(|&(x, y)| self.p(x, y)).collect(),
            fill,
            self.stroke(color, width),
        ));
    }
    fn arc(
        &self,
        center: (f32, f32),
        radius: f32,
        start: f32,
        end: f32,
        color: Color32,
        width: f32,
    ) {
        let points = (0..=24)
            .map(|i| {
                let angle = start + (end - start) * i as f32 / 24.;
                self.p(
                    center.0 + radius * angle.cos(),
                    center.1 + radius * angle.sin(),
                )
            })
            .collect();
        self.painter
            .add(Shape::line(points, self.stroke(color, width)));
    }
    fn star(&self, center: (f32, f32), radius: f32, fill: Color32) {
        self.line(
            &[(center.0 - radius, center.1), (center.0 + radius, center.1)],
            fill,
            1.7,
        );
        self.line(
            &[(center.0, center.1 - radius), (center.0, center.1 + radius)],
            fill,
            1.7,
        );
    }
}
