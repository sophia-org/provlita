//! Original bounded vector artwork, recorded directly into the GPU scene.
//! No font glyphs, image decoder, filesystem theme, or CPU raster step.
use masonry::{
    imaging::{Painter, record::Scene},
    kurbo::{BezPath, Circle, Line, Rect, Size, Stroke},
    peniko::Color,
};

pub(super) fn draw(scene: &mut Scene, size: Size, icon: &str, available: bool) {
    // Fit a 30x28 logical drawing to its actual retained canvas allocation.
    let sx = size.width / 30.0;
    let sy = size.height / 28.0;
    let rect = |x0, y0, x1, y1| Rect::new(x0 * sx, y0 * sy, x1 * sx, y1 * sy);
    let bright = if available {
        Color::from_rgb8(221, 232, 240)
    } else {
        Color::from_rgb8(120, 127, 134)
    };
    let accent = if available {
        Color::from_rgb8(111, 181, 210)
    } else {
        Color::from_rgb8(88, 101, 110)
    };
    let mut painter = Painter::new(scene);
    match icon {
        "terminal" => {
            painter.fill(rect(2.0, 4.0, 28.0, 25.0), bright).draw();
            painter
                .fill(rect(3.0, 8.0, 27.0, 24.0), Color::from_rgb8(20, 27, 33))
                .draw();
            painter.fill(rect(4.0, 5.0, 6.0, 7.0), accent).draw();
            let mut chevron = BezPath::new();
            chevron.move_to((7.0 * sx, 12.0 * sy));
            chevron.line_to((11.0 * sx, 16.0 * sy));
            chevron.line_to((7.0 * sx, 20.0 * sy));
            painter
                .stroke(&chevron, &Stroke::new(1.8 * sx.min(sy)), accent)
                .draw();
            painter.fill(rect(14.0, 19.0, 21.0, 21.0), bright).draw();
        }
        "browser" => {
            painter
                .fill(
                    Circle::new((15.0 * sx, 14.0 * sy), 11.0 * sx.min(sy)),
                    accent,
                )
                .draw();
            let stroke = Stroke::new(1.1 * sx.min(sy));
            for y in [9.0, 19.0] {
                painter
                    .stroke(
                        Line::new((6.0 * sx, y * sy), (24.0 * sx, y * sy)),
                        &stroke,
                        bright,
                    )
                    .draw();
            }
            let mut meridian = BezPath::new();
            meridian.move_to((15.0 * sx, 3.0 * sy));
            meridian.curve_to(
                (6.0 * sx, 9.0 * sy),
                (6.0 * sx, 19.0 * sy),
                (15.0 * sx, 25.0 * sy),
            );
            meridian.curve_to(
                (24.0 * sx, 19.0 * sy),
                (24.0 * sx, 9.0 * sy),
                (15.0 * sx, 3.0 * sy),
            );
            painter.stroke(&meridian, &stroke, bright).draw();
        }
        _ => {
            painter.fill(rect(3.0, 5.0, 14.0, 11.0), bright).draw();
            painter.fill(rect(3.0, 9.0, 27.0, 24.0), accent).draw();
            painter.fill(rect(4.0, 11.0, 26.0, 13.0), bright).draw();
        }
    }
}
