use std::cell::Cell;
use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{COLORREF, HWND, LRESULT, RECT};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED};
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_SETFONT};
use windows_sys::core::{PCWSTR, w};

use crate::text_of;

pub const WIDTH: i32 = 668;
pub const HEIGHT: i32 = 392;

const fn rgb(hex: u32) -> COLORREF {
    ((hex & 0xff0000) >> 16) | (hex & 0x00ff00) | ((hex & 0x0000ff) << 16)
}

struct Palette {
    background: COLORREF,
    card: COLORREF,
    field: COLORREF,
    border: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    primary: COLORREF,
    on_primary: COLORREF,
    selected: COLORREF,
    success: COLORREF,
    success_background: COLORREF,
}

const LIGHT: Palette = Palette {
    background: rgb(0xf1f4f9),
    card: rgb(0xffffff),
    field: rgb(0xf7f9fc),
    border: rgb(0xcbd5e3),
    text: rgb(0x17243b),
    muted: rgb(0x4f6078),
    primary: rgb(0x2850c7),
    on_primary: rgb(0xffffff),
    selected: rgb(0xe0e9ff),
    success: rgb(0x155939),
    success_background: rgb(0xe1f4e9),
};

const DARK: Palette = Palette {
    background: rgb(0x101622),
    card: rgb(0x1b2435),
    field: rgb(0x131c2b),
    border: rgb(0x3c4b64),
    text: rgb(0xf0f4fc),
    muted: rgb(0xaebbd0),
    primary: rgb(0xa2b8ff),
    on_primary: rgb(0x142444),
    selected: rgb(0x293b63),
    success: rgb(0x9aebba),
    success_background: rgb(0x1c3d34),
};

#[derive(Clone, Copy)]
pub enum Status {
    Neutral,
    Match,
    Mismatch,
}

#[derive(Clone, Copy)]
pub enum Button {
    Standard,
    Primary,
    Tab(bool),
    Theme,
}

pub struct Brush(HBRUSH);

impl Brush {
    fn new(color: COLORREF) -> io::Result<Self> {
        let brush = unsafe { CreateSolidBrush(color) };
        if brush.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(brush))
    }
}

impl Drop for Brush {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.0) };
    }
}

struct Theme {
    colors: Palette,
    background: Brush,
    card: Brush,
    field: Brush,
}

impl Theme {
    fn new(colors: Palette) -> io::Result<Self> {
        Ok(Self {
            background: Brush::new(colors.background)?,
            card: Brush::new(colors.card)?,
            field: Brush::new(colors.field)?,
            colors,
        })
    }
}

pub struct Font(HFONT);

impl Font {
    fn new(face: PCWSTR, height: i32, weight: u32) -> io::Result<Self> {
        let font = unsafe {
            CreateFontW(
                -height, 0, 0, 0, weight as i32, 0, 0, 0, DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32, CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32, face,
            )
        };
        if font.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(font))
    }

    pub fn apply(&self, control: HWND) {
        unsafe { SendMessageW(control, WM_SETFONT, self.0 as usize, 0) };
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.0) };
    }
}

pub struct Appearance {
    dark: Cell<bool>,
    light: Theme,
    night: Theme,
    pub body: Font,
    pub strong: Font,
    pub title: Font,
    pub mono: Font,
}

impl Appearance {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            dark: Cell::new(false),
            light: Theme::new(LIGHT)?,
            night: Theme::new(DARK)?,
            body: Font::new(w!("Segoe UI"), 15, FW_NORMAL)?,
            strong: Font::new(w!("Segoe UI"), 15, FW_SEMIBOLD)?,
            title: Font::new(w!("Segoe UI"), 28, FW_SEMIBOLD)?,
            mono: Font::new(w!("Consolas"), 13, FW_NORMAL)?,
        })
    }

    pub fn toggle(&self, parent: HWND) -> &'static str {
        let dark = !self.dark.get();
        self.dark.set(dark);
        let enabled = i32::from(dark);
        unsafe {
            DwmSetWindowAttribute(
                parent,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                (&enabled as *const i32).cast(),
                size_of::<i32>() as u32,
            );
        }
        if dark { "Switch to light mode" } else { "Switch to dark mode" }
    }

    fn theme(&self) -> &Theme {
        if self.dark.get() { &self.night } else { &self.light }
    }

    pub fn color_control(&self, dc: HDC, field: bool, header: bool, muted: bool) -> LRESULT {
        let theme = self.theme();
        let (background, brush) = if field {
            (theme.colors.field, &theme.field)
        } else if header {
            (theme.colors.background, &theme.background)
        } else {
            (theme.colors.card, &theme.card)
        };
        unsafe {
            SetBkColor(dc, background);
            SetTextColor(dc, if muted { theme.colors.muted } else { theme.colors.text });
        }
        brush.0 as LRESULT
    }

    pub fn paint_window(&self, parent: HWND, validating: bool) {
        let mut paint = PAINTSTRUCT::default();
        unsafe {
            let dc = BeginPaint(parent, &mut paint);
            self.paint_background(dc, &paint.rcPaint, validating);
            EndPaint(parent, &paint);
        }
    }

    pub fn paint_background(&self, dc: HDC, bounds: &RECT, validating: bool) {
        let theme = self.theme();
        let colors = &theme.colors;
        unsafe { FillRect(dc, bounds, theme.background.0) };
        rounded(dc, &rect(24, 143, 644, 370), 16, colors.card, colors.border);
        rounded(dc, &rect(44, 183, 530, 223), 10, colors.field, colors.border);
        let right = if validating { 624 } else { 530 };
        rounded(dc, &rect(44, 258, right, 298), 10, colors.field, colors.border);
    }

    pub fn paint_button(&self, item: &DRAWITEMSTRUCT, kind: Button) {
        let theme = self.theme();
        let colors = &theme.colors;
        let disabled = item.itemState & ODS_DISABLED != 0;
        let pressed = item.itemState & ODS_SELECTED != 0;
        let outside = match kind {
            Button::Theme | Button::Tab(_) => &theme.background,
            _ => &theme.card,
        };
        let (fill, ink, outline) = match kind {
            Button::Primary if !disabled && !pressed => (colors.primary, colors.on_primary, colors.primary),
            Button::Tab(true) | Button::Primary if !disabled => (colors.selected, colors.primary, colors.primary),
            Button::Tab(true) => (colors.selected, colors.muted, colors.border),
            Button::Tab(false) if !pressed => (colors.background, colors.muted, colors.background),
            _ if pressed => (colors.selected, colors.primary, colors.primary),
            _ if disabled => (colors.field, colors.muted, colors.border),
            _ => (colors.card, colors.text, colors.border),
        };
        unsafe { FillRect(item.hDC, &item.rcItem, outside.0) };
        rounded(item.hDC, &item.rcItem, 10, fill, outline);
        if matches!(kind, Button::Theme) {
            draw_theme_icon(item.hDC, &item.rcItem, ink, fill, self.dark.get());
        } else {
            draw_text(item.hDC, item.hwndItem, &item.rcItem, &self.strong, ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        if item.itemState & ODS_FOCUS != 0 {
            let mut focus = item.rcItem;
            unsafe {
                InflateRect(&mut focus, -4, -4);
                DrawFocusRect(item.hDC, &focus);
            }
        }
    }

    pub fn paint_status(&self, item: &DRAWITEMSTRUCT, status: Status) {
        let theme = self.theme();
        let colors = &theme.colors;
        let dc = item.hDC;
        unsafe { FillRect(dc, &item.rcItem, theme.card.0) };
        if matches!(status, Status::Neutral) {
            draw_text(dc, item.hwndItem, &item.rcItem, &self.body, colors.muted, DT_WORDBREAK);
            return;
        }
        let (fill, ink, width) = match status {
            Status::Match => (colors.success_background, colors.success, 119),
            Status::Mismatch => (rgb(0xfbe6e9), rgb(0x561f30), 159),
            Status::Neutral => unreachable!(),
        };
        let bounds = rect(item.rcItem.left, item.rcItem.top, item.rcItem.left + width, item.rcItem.top + 27);
        rounded(dc, &bounds, 10, fill, fill);
        draw_text(dc, item.hwndItem, &bounds, &self.strong, ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT { left, top, right, bottom }
}

fn rounded(dc: HDC, bounds: &RECT, radius: i32, fill: COLORREF, outline: COLORREF) {
    unsafe {
        let brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        let pen = SelectObject(dc, GetStockObject(DC_PEN));
        SetDCBrushColor(dc, fill);
        SetDCPenColor(dc, outline);
        RoundRect(dc, bounds.left, bounds.top, bounds.right, bounds.bottom, radius, radius);
        SelectObject(dc, pen);
        SelectObject(dc, brush);
    }
}

fn draw_text(dc: HDC, control: HWND, bounds: &RECT, font: &Font, color: COLORREF, flags: u32) {
    let text = text_of(control);
    let mut bounds = *bounds;
    unsafe {
        let previous_font = SelectObject(dc, font.0);
        let previous_color = SetTextColor(dc, color);
        let previous_mode = SetBkMode(dc, TRANSPARENT as i32);
        DrawTextW(dc, text.as_ptr(), text.len() as i32, &mut bounds, flags | DT_NOPREFIX);
        SetBkMode(dc, previous_mode);
        SetTextColor(dc, previous_color);
        SelectObject(dc, previous_font);
    }
}

fn draw_theme_icon(dc: HDC, bounds: &RECT, ink: COLORREF, fill: COLORREF, dark: bool) {
    let x = (bounds.left + bounds.right) / 2;
    let y = (bounds.top + bounds.bottom) / 2;
    if !dark {
        rounded(dc, &rect(x - 7, y - 8, x + 8, y + 7), 15, ink, ink);
        rounded(dc, &rect(x - 2, y - 10, x + 10, y + 2), 12, fill, fill);
        return;
    }
    rounded(dc, &rect(x - 4, y - 4, x + 5, y + 5), 9, ink, ink);
    unsafe {
        let pen = SelectObject(dc, GetStockObject(DC_PEN));
        SetDCPenColor(dc, ink);
        for (dx, dy) in [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)] {
            let inner = if dx != 0 && dy != 0 { 5 } else { 6 };
            let outer = inner + 3;
            MoveToEx(dc, x + dx * inner, y + dy * inner, null_mut());
            LineTo(dc, x + dx * outer, y + dy * outer);
        }
        SelectObject(dc, pen);
    }
}
