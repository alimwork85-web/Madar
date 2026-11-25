//! Direct2D drawing of the wheel into a layered window.

use super::*;
use crate::icons;
use windows::Win32::Foundation::{COLORREF, HWND, SIZE};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Result, w};

/// The `windows` crate does not export its point type, so borrow it from a struct.
macro_rules! pt {
    ($x:expr, $y:expr) => {{
        let mut p = D2D1_ELLIPSE::default().point;
        p.X = $x;
        p.Y = $y;
        p
    }};
}

/// Room around the plate for antialiased edges.
const HALF: f32 = PLATE_R + 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub plate: u32,
    pub slot: u32,
    pub text: u32,
    pub accent: u32,
    pub on_accent: u32,
    pub disabled: u32,
    /// Fill of a free place.
    pub empty: u32,
    /// High Contrast draws slot outlines because its fills are equal.
    pub outline: bool,
}

pub fn theme(name: &str) -> Palette {
    let (plate, slot, text, accent, on_accent) = match name {
        "Space Gray" => (0x1B1B1E, 0x2F2F33, 0xF2F2F4, 0xD6D6DB, 0x1B1B1E),
        "Ember" => (0x1B100B, 0x40211A, 0xFBEDE6, 0xFF9A6B, 0x2A0F05),
        "Grape" => (0x150F22, 0x33234F, 0xF2ECFB, 0xC7A6FF, 0x1E0F38),
        "Solar" => (0x1B140A, 0x47341A, 0xFBF3E2, 0xF4C04E, 0x2A1C04),
        "Meadow" => (0x0B1A14, 0x1D4034, 0xE8F5EE, 0x7FDDB0, 0x06251A),
        "Aqua" => (0xFAFCFF, 0xE6EAF0, 0x17202E, 0x0A66C2, 0xFFFFFF),
        "Porcelain" => (0xEEF2F5, 0xCFD9E2, 0x1C2833, 0x2F5D7C, 0xFFFFFF),
        _ => (0x0F151C, 0x1E2C38, 0xEAF2F8, 0x7CC4F0, 0x0B1A26),
    };
    Palette {
        plate,
        slot,
        text,
        accent,
        on_accent,
        disabled: mix(text, plate, 0.6),
        // Aqua's slot is already close to its plate.
        empty: mix(slot, plate, if name == "Aqua" { 0.8 } else { 0.35 }),
        outline: false,
    }
}

impl Palette {
    /// The two ends of a slot color's gradient, top left first. Neither end
    /// has less contrast with the text than `c` itself.
    fn slot_tones(&self, c: u32) -> (u32, u32) {
        if luminance(self.plate) > luminance(self.slot) {
            (mix(c, self.plate, 0.65), c)
        } else {
            (c, mix(c, self.plate, 0.45))
        }
    }

    /// The same for the hub.
    fn hub_tones(&self) -> (u32, u32) {
        if luminance(self.plate) > luminance(self.slot) {
            (mix(self.plate, 0xFFFFFF, 0.5), self.plate)
        } else {
            (self.plate, mix(self.plate, 0, 0.65))
        }
    }

    fn rim(&self) -> u32 {
        mix(self.accent, self.plate, 0.25)
    }
}

pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (a, b) = ((a >> s & 255) as f32, (b >> s & 255) as f32);
        ((a * t + b * (1.0 - t)).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// The theme, unless Windows High Contrast is on.
pub fn palette(name: &str) -> Palette {
    let mut hc = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some((&raw mut hc).cast()),
            Default::default(),
        )
    };
    if ok.is_err() || (hc.dwFlags & HCF_HIGHCONTRASTON).0 == 0 {
        return theme(name);
    }
    let sys = |i| {
        let c = unsafe { GetSysColor(i) };
        (c & 255) << 16 | c & 0xFF00 | c >> 16 & 255
    };
    Palette {
        plate: sys(COLOR_WINDOW),
        slot: sys(COLOR_WINDOW),
        text: sys(COLOR_WINDOWTEXT),
        accent: sys(COLOR_HIGHLIGHT),
        on_accent: sys(COLOR_HIGHLIGHTTEXT),
        disabled: sys(COLOR_GRAYTEXT),
        empty: sys(COLOR_WINDOW),
        outline: true,
    }
}

/// Everything one frame shows.
pub struct Scene<'a> {
    pub slots: &'a [Slot],
    pub hot: Option<usize>,
    pub pressed: Option<usize>,
    /// False: icons alone, without names and digits.
    pub names: bool,
    pub hub: &'a str,
    pub hub_hot: bool,
    pub palette: Palette,
    /// 0 to 1, opening animation.
    pub progress: f32,
}

/// Memory bitmap the frame is drawn into.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    side: i32,
}

impl Surface {
    fn new(side: i32) -> Result<Self> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side,
                biHeight: -side,
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        unsafe {
            let dc = CreateCompatibleDC(None);
            match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) => {
                    let old = SelectObject(dc, bitmap.into());
                    Ok(Self {
                        dc,
                        bitmap,
                        old,
                        side,
                    })
                }
                Err(e) => {
                    let _ = DeleteDC(dc);
                    Err(e)
                }
            }
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

pub struct Painter {
    d2d: ID2D1Factory,
    write: IDWriteFactory,
    icon: IDWriteTextFormat,
    big_icon: IDWriteTextFormat,
    label: IDWriteTextFormat,
    digit: IDWriteTextFormat,
    target: Option<ID2D1DCRenderTarget>,
    surface: Option<Surface>,
}

impl Painter {
    pub fn new() -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let format = |family, size| -> Result<IDWriteTextFormat> {
                let f = write.CreateTextFormat(
                    family,
                    None,
                    DWRITE_FONT_WEIGHT_REGULAR,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )?;
                f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                Ok(f)
            };
            let icon = format(w!("Segoe Fluent Icons"), icons::WITH_NAME)?;
            let big_icon = format(w!("Segoe Fluent Icons"), icons::ALONE)?;
            let label = format(w!("Segoe UI Variable Text"), 13.0)?;
            let digit = format(w!("Segoe UI Variable Text"), 12.0)?;
            let dots = write.CreateEllipsisTrimmingSign(&label)?;
            let trim = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                ..Default::default()
            };
            label.SetTrimming(&trim, &dots)?;
            Ok(Self {
                d2d,
                write,
                icon,
                big_icon,
                label,
                digit,
                target: None,
                surface: None,
            })
        }
    }

    /// Draws `s` and shows it centered on `center` at `scale` (1 is 96 DPI).
    pub fn present(&mut self, hwnd: HWND, center: POINT, scale: f32, s: &Scene) -> Result<()> {
        let side = (HALF * 2.0 * scale).ceil() as i32;
        let drawn = self.draw(side, scale, s);
        if drawn.is_err() {
            // A lost device is rebuilt by the next frame.
            self.target = None;
            drawn?;
        }
        let Some(surface) = &self.surface else {
            return Ok(());
        };
        let pos = POINT {
            x: center.x - side / 2,
            y: center.y - side / 2,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            SourceConstantAlpha: (s.progress.clamp(0.0, 1.0) * 255.0) as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
            ..Default::default()
        };
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                None,
                Some(&pos),
                Some(&SIZE { cx: side, cy: side }),
                Some(surface.dc),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
    }

    fn draw(&mut self, side: i32, scale: f32, s: &Scene) -> Result<()> {
        if self.surface.as_ref().is_none_or(|x| x.side != side) {
            self.surface = Some(Surface::new(side)?);
        }
        let rt = match &self.target {
            Some(t) => t.clone(),
            None => {
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    ..Default::default()
                };
                let t = unsafe { self.d2d.CreateDCRenderTarget(&props)? };
                self.target.insert(t).clone()
            }
        };
        let Some(surface) = &self.surface else {
            return Ok(());
        };
        unsafe {
            let all = RECT {
                left: 0,
                top: 0,
                right: side,
                bottom: side,
            };
            // Otherwise the system DPI would scale the drawing a second time.
            rt.SetDpi(96.0, 96.0);
            rt.BindDC(surface.dc, &all)?;
            rt.BeginDraw();
            let mut m = D2D1_DRAWING_STATE_DESCRIPTION::default().transform;
            let zoom = scale * (0.96 + 0.04 * s.progress);
            (m.M11, m.M22) = (zoom, zoom);
            (m.M31, m.M32) = (side as f32 / 2.0, side as f32 / 2.0);
            rt.SetTransform(&m);
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            rt.Clear(Some(&D2D1_COLOR_F::default()));
            let drawn = self.frame(&rt, s);
            rt.EndDraw(None, None)?;
            drawn
        }
    }

    fn frame(&self, rt: &ID2D1DCRenderTarget, s: &Scene) -> Result<()> {
        let p = s.palette;
        let brush = |c| unsafe { rt.CreateSolidColorBrush(&color(c), None) };
        let (text, on_accent, disabled) = (brush(p.text)?, brush(p.on_accent)?, brush(p.disabled)?);
        // From top left to bottom right of a circle of radius `r`.
        let shade = |(from, to), r: f32| {
            let d = r * std::f32::consts::FRAC_1_SQRT_2;
            let stop = |position, c| D2D1_GRADIENT_STOP {
                position,
                color: color(c),
            };
            let ends = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: pt!(-d, -d),
                endPoint: pt!(d, d),
            };
            unsafe {
                let stops = rt.CreateGradientStopCollection(
                    &[stop(0.0, from), stop(1.0, to)],
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                )?;
                rt.CreateLinearGradientBrush(&ends, None, &stops)
            }
        };
        let normal = shade(p.slot_tones(p.slot), OUTER_R)?;
        let faint = shade(p.slot_tones(p.empty), OUTER_R)?;
        let disc = |r| D2D1_ELLIPSE {
            point: pt!(0.0, 0.0),
            radiusX: r,
            radiusY: r,
        };
        unsafe {
            rt.FillEllipse(&disc(PLATE_R), &brush(p.plate)?);
            if p.outline {
                rt.DrawEllipse(&disc(PLATE_R - 0.5), &text, 1.0, None);
            } else {
                rt.DrawEllipse(&disc(PLATE_R - 0.5), &brush(p.rim())?, 1.0, None);
            }
        }
        let n = s.slots.len();
        for (i, slot) in s.slots.iter().enumerate() {
            if slot.target == Target::Empty {
                unsafe { rt.FillGeometry(&self.slot_shape(i, n, 0.0)?, &faint, None) };
                continue;
            }
            let off = slot.target == Target::Disabled;
            let hot = s.hot == Some(i);
            let (fill, ink) = match (off, hot) {
                (true, _) => (p.plate, &disabled),
                (false, true) => (p.accent, &on_accent),
                (false, false) => (p.slot, &text),
            };
            unsafe {
                let shape = self.slot_shape(i, n, 0.0)?;
                if off || hot {
                    rt.FillGeometry(&shape, &brush(fill)?, None);
                } else {
                    rt.FillGeometry(&shape, &normal, None);
                }
                // Keyboard users must still see where they are on a disabled slot.
                if p.outline || (off && hot) {
                    rt.DrawGeometry(&self.slot_shape(i, n, 0.5)?, ink, 1.0, None);
                }
                if s.pressed == Some(i) && !off {
                    rt.DrawGeometry(&self.slot_shape(i, n, 4.0)?, &on_accent, 2.0, None);
                }
            }
            if s.names {
                self.slot_text(rt, slot, i, n, ink, fill)?;
            } else {
                self.slot_icon(rt, slot, i, n, ink, fill)?;
            }
        }
        let hub = self.layout(s.hub, &self.label, 92.0, 70.0)?;
        unsafe {
            if s.hub_hot {
                rt.FillEllipse(&disc(HUB_R), &brush(p.accent)?);
            } else if p.outline {
                rt.DrawEllipse(&disc(HUB_R - 0.5), &text, 1.0, None);
            } else {
                rt.FillEllipse(&disc(HUB_R), &shade(p.hub_tones(), HUB_R)?);
            }
            let ink = if s.hub_hot { &on_accent } else { &text };
            rt.DrawTextLayout(pt!(-46.0, -35.0), &hub, ink, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        Ok(())
    }

    /// The icon alone, larger.
    fn slot_icon(
        &self,
        rt: &ID2D1DCRenderTarget,
        slot: &Slot,
        i: usize,
        n: usize,
        ink: &ID2D1SolidColorBrush,
        fill: u32,
    ) -> Result<()> {
        let a = std::f32::consts::TAU * i as f32 / n as f32;
        let (x, y) = (ICON_R * a.sin(), -ICON_R * a.cos());
        if let Some(image) = &slot.image {
            return self.image(rt, image, x, y, icons::ALONE, fill);
        }
        let glyph = slot.icon.encode_utf8(&mut [0; 4]).to_string();
        let icon = self.layout(&glyph, &self.big_icon, 40.0, 40.0)?;
        let plain = D2D1_DRAW_TEXT_OPTIONS_NONE;
        unsafe { rt.DrawTextLayout(pt!(x - 20.0, y - 20.0), &icon, ink, plain) };
        Ok(())
    }

    /// Icon with its digit, then the label, as one block centered on the slot.
    fn slot_text(
        &self,
        rt: &ID2D1DCRenderTarget,
        slot: &Slot,
        i: usize,
        n: usize,
        ink: &ID2D1SolidColorBrush,
        fill: u32,
    ) -> Result<()> {
        let a = std::f32::consts::TAU * i as f32 / n as f32;
        let (x, y) = (ICON_R * a.sin(), -ICON_R * a.cos());
        let width: f32 = match n {
            // Slots at the sides leave more room across than those at the top.
            8 => 72.0 + 14.0 * a.sin().abs(),
            7 => 84.0,
            _ => 92.0,
        };
        let label = self.layout(&slot.label, &self.label, width, 36.0)?;
        let mut size = DWRITE_TEXT_METRICS::default();
        unsafe { label.GetMetrics(&mut size)? };
        let text_h = size.height.min(36.0);
        let top = y - (24.0 + 2.0 + text_h) / 2.0;
        let icon = self.layout(slot.icon.encode_utf8(&mut [0; 4]), &self.icon, 30.0, 24.0)?;
        let digit = self.layout(&(i + 1).to_string(), &self.digit, 16.0, 24.0)?;
        unsafe {
            let plain = D2D1_DRAW_TEXT_OPTIONS_NONE;
            match &slot.image {
                Some(image) => self.image(rt, image, x, top + 12.0, icons::WITH_NAME, fill)?,
                None => rt.DrawTextLayout(pt!(x - 15.0, top), &icon, ink, plain),
            }
            rt.DrawTextLayout(pt!(x + 15.0, top), &digit, ink, plain);
            // The layout centers its text in 36 px, so shift up by half the spare room.
            let label_top = top + 26.0 - (36.0 - text_h) / 2.0;
            rt.DrawTextLayout(pt!(x - width / 2.0, label_top), &label, ink, plain);
        }
        Ok(())
    }

    /// Program icon centered on a point, with a base when it would fade into `fill`.
    fn image(
        &self,
        rt: &ID2D1DCRenderTarget,
        image: &icons::Icon,
        x: f32,
        y: f32,
        size: f32,
        fill: u32,
    ) -> Result<()> {
        let half = size / 2.0;
        let mut m = D2D1_DRAWING_STATE_DESCRIPTION::default().transform;
        unsafe { rt.GetTransform(&mut m) };
        // Whole device pixels, so that the icon stays sharp.
        let snap = |v: f32, shift: f32| ((v * m.M11 + shift).round() - shift) / m.M11;
        let (left, top) = (snap(x - half, m.M31), snap(y - half, m.M32));
        let rect = |grow: f32| D2D_RECT_F {
            left: left - grow,
            top: top - grow,
            right: left + half * 2.0 + grow,
            bottom: top + half * 2.0 + grow,
        };
        let size = D2D_SIZE_U {
            width: image.width,
            height: image.height,
        };
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        unsafe {
            if let Some(c) = image.base(fill) {
                let round = D2D1_ROUNDED_RECT {
                    rect: rect(2.0),
                    radiusX: 5.0,
                    radiusY: 5.0,
                };
                rt.FillRoundedRectangle(&round, &rt.CreateSolidColorBrush(&color(c), None)?);
            }
            let data = Some(image.pixels.as_ptr().cast());
            let bitmap = rt.CreateBitmap(size, data, image.width * 4, &props)?;
            let linear = D2D1_BITMAP_INTERPOLATION_MODE_LINEAR;
            rt.DrawBitmap(&bitmap, Some(&rect(0.0)), 1.0, linear, None);
        }
        Ok(())
    }

    fn layout(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        w: f32,
        h: f32,
    ) -> Result<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let layout = unsafe { self.write.CreateTextLayout(&wide, format, w, h)? };
        // Hebrew and Arabic blocks: wrap and trim from the right.
        if wide.iter().any(|c| (0x590..0x900).contains(c)) {
            unsafe { layout.SetReadingDirection(DWRITE_READING_DIRECTION_RIGHT_TO_LEFT)? };
        }
        Ok(layout)
    }

    /// Ring sector whose straight sides stay parallel to its neighbors' sides.
    fn slot_shape(&self, i: usize, n: usize, inset: f32) -> Result<ID2D1PathGeometry> {
        let step = std::f32::consts::TAU / n as f32;
        let mid = step * i as f32;
        let (outer, inner) = (OUTER_R - inset, INNER_R + inset);
        let half = |r: f32| step / 2.0 - ((GAP / 2.0 + inset) / r).asin();
        let on = |r: f32, a: f32| pt!(r * a.sin(), -r * a.cos());
        let arc = |r: f32, a: f32, sweep| D2D1_ARC_SEGMENT {
            point: on(r, a),
            size: D2D_SIZE_F {
                width: r,
                height: r,
            },
            rotationAngle: 0.0,
            sweepDirection: sweep,
            arcSize: if n == 1 {
                D2D1_ARC_SIZE_LARGE
            } else {
                D2D1_ARC_SIZE_SMALL
            },
        };
        unsafe {
            let path = self.d2d.CreatePathGeometry()?;
            let sink = path.Open()?;
            let (cw, ccw) = (
                D2D1_SWEEP_DIRECTION_CLOCKWISE,
                D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
            );
            sink.BeginFigure(on(outer, mid - half(outer)), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddArc(&arc(outer, mid + half(outer), cw));
            sink.AddLine(on(inner, mid + half(inner)));
            sink.AddArc(&arc(inner, mid - half(inner), ccw));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            Ok(path)
        }
    }
}

fn color(c: u32) -> D2D1_COLOR_F {
    let ch = |s: u32| (c >> s & 255) as f32 / 255.0;
    D2D1_COLOR_F {
        r: ch(16),
        g: ch(8),
        b: ch(0),
        a: 1.0,
    }
}

/// WCAG relative luminance of a color.
pub fn luminance(c: u32) -> f32 {
    let ch = |s: u32| {
        let v = (c >> s & 255) as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
}

/// WCAG contrast of two luminances.
pub fn contrast_of(x: f32, y: f32) -> f32 {
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: u32, b: u32) -> f32 {
        contrast_of(luminance(a), luminance(b))
    }

    #[test]
    fn every_theme_is_readable() {
        for name in crate::config::THEMES {
            let p = theme(name);
            // WCAG exempts disabled controls; 60% text lands at 4.3 on Aqua.
            for (what, a, b, least) in [
                ("text", p.text, p.slot, 4.5),
                ("hub", p.text, p.plate, 4.5),
                ("accent", p.on_accent, p.accent, 4.5),
                ("disabled", p.disabled, p.plate, 3.0),
            ] {
                assert!(contrast(a, b) >= least, "{name} {what} {}", contrast(a, b));
            }
        }
    }

    #[test]
    fn gradients_stay_readable() {
        for name in crate::config::THEMES {
            let p = theme(name);
            let ((a, b), (c, d)) = (p.slot_tones(p.slot), p.hub_tones());
            for (tone, flat) in [(a, p.slot), (b, p.slot), (c, p.plate), (d, p.plate)] {
                let (now, before) = (contrast(p.text, tone), contrast(p.text, flat));
                assert!(now >= before && now >= 4.5, "{name} {tone:06X} {now}");
            }
            assert!(a != b && c != d && p.rim() != p.plate, "{name}");
        }
    }

    #[test]
    fn free_places_show_on_the_plate() {
        for name in crate::config::THEMES {
            let p = theme(name);
            let (a, b) = p.slot_tones(p.empty);
            let c = contrast(a, p.plate).max(contrast(b, p.plate));
            assert!(c >= 1.07, "{name} {c}");
        }
    }

    #[test]
    fn themes_are_distinct_and_unknown_is_midnight() {
        let all = crate::config::THEMES.map(theme);
        for (i, a) in all.iter().enumerate() {
            assert!(!all[i + 1..].contains(a));
        }
        assert_eq!(theme("nope"), theme("Midnight"));
    }
}
