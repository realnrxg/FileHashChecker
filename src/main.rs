#![windows_subsystem = "windows"]

mod appearance;

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::{
    HDC, InvalidateRect, RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE, RedrawWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, SetFocus, VK_A, VK_CONTROL, VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT,
};
use windows_sys::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::{PCWSTR, w};

use appearance::{Appearance, Button, Font, HEIGHT, Status, WIDTH};

const TITLE: PCWSTR = w!("File Hash Checker");
const CLASS: PCWSTR = w!("FileHashCheckerWindow");
const BROWSE: usize = 100;
const ACTION: usize = 101;
const COPY: usize = 102;
const CHECKER_TAB: usize = 103;
const VALIDATOR_TAB: usize = 104;
const THEME_TOGGLE: usize = 105;
const STATUS: usize = 106;
const TIMER: usize = 1;
const SS_OWNERDRAW: u32 = 0x000d;

struct Job {
    expected: Option<String>,
    result: Receiver<io::Result<String>>,
}

struct Window {
    appearance: Appearance,
    validating: Cell<bool>,
    status_kind: Cell<Status>,
    title: HWND,
    hint: HWND,
    checker_tab: HWND,
    validator_tab: HWND,
    theme_toggle: HWND,
    path: HWND,
    browse: HWND,
    expected_label: HWND,
    expected: HWND,
    action: HWND,
    digest_label: HWND,
    digest: HWND,
    copy: HWND,
    status: HWND,
    job: RefCell<Option<Job>>,
}

fn hash_file(path: PathBuf) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut sha256 = Sha256::new();
    io::copy(&mut file, &mut sha256)?;
    Ok(format!("{:x}", sha256.finalize()))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn text_of(control: HWND) -> Vec<u16> {
    unsafe {
        let mut text = vec![0; GetWindowTextLengthW(control) as usize + 1];
        let length = GetWindowTextW(control, text.as_mut_ptr(), text.len() as i32);
        text.truncate(length as usize);
        text
    }
}

fn set_text(control: HWND, text: &str) {
    unsafe { SetWindowTextW(control, wide(text).as_ptr()) };
}

fn control(
    parent: HWND,
    class: PCWSTR,
    text: PCWSTR,
    bounds: [i32; 4],
    style: u32,
    id: usize,
    font: &Font,
) -> io::Result<HWND> {
    let [x, y, width, height] = bounds;
    let handle = unsafe {
        CreateWindowExW(
            0,
            class,
            text,
            WS_CHILD | WS_VISIBLE | style,
            x,
            y,
            width,
            height,
            parent,
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        )
    };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    font.apply(handle);
    Ok(handle)
}

impl Window {
    fn new(parent: HWND) -> io::Result<Self> {
        let appearance = Appearance::new()?;
        let label = |text, bounds| {
            control(parent, w!("STATIC"), text, bounds, 0, 0, &appearance.strong)
        };
        let edit = |bounds, style, font| {
            control(
                parent,
                w!("EDIT"),
                null(),
                bounds,
                WS_TABSTOP | ES_AUTOHSCROLL as u32 | style,
                0,
                font,
            )
        };
        let button = |text, bounds, id| {
            control(
                parent, w!("BUTTON"), text, bounds, WS_TABSTOP | BS_OWNERDRAW as u32, id,
                &appearance.strong,
            )
        };

        let title = control(
            parent, w!("STATIC"), TITLE, [24, 19, 500, 36], 0, 0, &appearance.title,
        )?;
        let hint = control(
            parent, w!("STATIC"), w!("Drop a file here, browse for one, or paste its path."),
            [25, 61, 540, 23], 0, 0, &appearance.body,
        )?;
        let checker_tab = button(w!("Hash Checker"), [24, 95, 139, 35], CHECKER_TAB)?;
        let validator_tab = button(w!("Hash Validator"), [171, 95, 145, 35], VALIDATOR_TAB)?;
        let theme_toggle = button(w!("Switch to dark mode"), [599, 24, 45, 38], THEME_TOGGLE)?;
        label(w!("File path"), [44, 160, 580, 20])?;
        let path = edit([55, 193, 464, 22], 0, &appearance.body)?;
        let browse = button(w!("Browse…"), [542, 183, 82, 40], BROWSE)?;
        let expected_label = label(w!("Expected SHA-256 hash"), [44, 235, 580, 20])?;
        let expected = edit([55, 269, 558, 21], 0, &appearance.mono)?;
        let digest_label = label(w!("SHA-256 hash"), [44, 235, 580, 20])?;
        let digest = edit([55, 269, 464, 21], ES_READONLY as u32, &appearance.mono)?;
        let copy = button(w!("Copy"), [542, 258, 82, 40], COPY)?;
        let action = button(w!("Calculate hash"), [44, 314, 140, 36], ACTION)?;
        let status = control(
            parent, w!("STATIC"), null(), [199, 319, 425, 44], SS_OWNERDRAW, STATUS,
            &appearance.body,
        )?;

        let window = Self {
            appearance,
            validating: Cell::new(false),
            status_kind: Cell::new(Status::Neutral),
            title,
            hint,
            checker_tab,
            validator_tab,
            theme_toggle,
            path,
            browse,
            expected_label,
            expected,
            action,
            digest_label,
            digest,
            copy,
            status,
            job: RefCell::new(None),
        };
        window.show_tab(parent);
        Ok(window)
    }

    fn validating(&self) -> bool {
        self.validating.get()
    }

    fn select_tab(&self, parent: HWND, validating: bool) {
        if self.job.borrow().is_none() && self.validating.replace(validating) != validating {
            self.show_tab(parent);
        }
    }

    fn show_tab(&self, parent: HWND) {
        let validating = self.validating();
        for (handle, visible) in [
            (self.expected_label, validating),
            (self.expected, validating),
            (self.digest_label, !validating),
            (self.digest, !validating),
            (self.copy, !validating),
        ] {
            unsafe { ShowWindow(handle, if visible { SW_SHOW } else { SW_HIDE }) };
        }
        set_text(
            self.action,
            if validating { "Validate hash" } else { "Calculate hash" },
        );
        self.clear_result();
        self.redraw(parent);
    }

    fn redraw(&self, parent: HWND) {
        unsafe {
            RedrawWindow(parent, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN)
        };
    }

    fn toggle_theme(&self, parent: HWND) {
        set_text(self.theme_toggle, self.appearance.toggle(parent));
        self.redraw(parent);
    }

    fn set_status(&self, text: &str, kind: Status) {
        self.status_kind.set(kind);
        set_text(self.status, text);
        unsafe { InvalidateRect(self.status, null(), 0) };
    }

    fn paint_item(&self, item: &DRAWITEMSTRUCT) {
        let button = match item.CtlID as usize {
            CHECKER_TAB => Button::Tab(!self.validating()),
            VALIDATOR_TAB => Button::Tab(self.validating()),
            THEME_TOGGLE => Button::Theme,
            BROWSE | COPY => Button::Standard,
            ACTION => Button::Primary,
            STATUS => {
                self.appearance.paint_status(item, self.status_kind.get());
                return;
            }
            _ => return,
        };
        self.appearance.paint_button(item, button);
    }

    fn clear_result(&self) {
        set_text(self.digest, "");
        self.set_status("", Status::Neutral);
        unsafe { EnableWindow(self.copy, 0) };
    }

    fn select_file(&self, parent: HWND, path: &[u16]) {
        unsafe { SetWindowTextW(self.path, path.as_ptr()) };
        if !self.validating() {
            self.start(parent);
        }
    }

    fn browse(&self, parent: HWND) {
        let mut path = vec![0; 32768];
        let mut dialog = OPENFILENAMEW {
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: parent,
            lpstrFilter: w!("Executables (*.exe)\0*.exe\0All files (*.*)\0*.*\0"),
            nFilterIndex: 1,
            lpstrFile: path.as_mut_ptr(),
            nMaxFile: path.len() as u32,
            Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR,
            ..Default::default()
        };
        unsafe {
            if GetOpenFileNameW(&mut dialog) != 0 {
                self.select_file(parent, &path);
            } else if CommDlgExtendedError() != 0 {
                self.clear_result();
                self.set_status("Could not open the file picker.", Status::Neutral);
            }
        }
    }

    fn drop_file(&self, parent: HWND, drop: HDROP) {
        unsafe {
            let count = DragQueryFileW(drop, u32::MAX, null_mut(), 0);
            if self.job.borrow().is_some() || count != 1 {
                DragFinish(drop);
                if self.job.borrow().is_none() {
                    self.clear_result();
                    self.set_status("Drop one file at a time.", Status::Neutral);
                }
                return;
            }
            let length = DragQueryFileW(drop, 0, null_mut(), 0);
            let mut path = vec![0; length as usize + 1];
            let copied = DragQueryFileW(drop, 0, path.as_mut_ptr(), length + 1);
            DragFinish(drop);
            if copied > 0 {
                self.select_file(parent, &path);
            }
        }
    }

    fn set_busy(&self, busy: bool) {
        for handle in [
            self.checker_tab, self.validator_tab, self.path, self.browse, self.expected, self.action,
        ] {
            unsafe { EnableWindow(handle, i32::from(!busy)) };
        }
    }

    fn start(&self, parent: HWND) {
        if self.job.borrow().is_some() {
            return;
        }
        self.clear_result();

        let raw_path = text_of(self.path);
        // Explorer's "Copy as path" surrounds the path with double quotes.
        let quote = u16::from(b'"');
        let path = raw_path
            .strip_prefix(&[quote])
            .and_then(|path| path.strip_suffix(&[quote]))
            .unwrap_or(&raw_path);
        if path.is_empty() {
            self.set_status("Choose a file first.", Status::Neutral);
            return;
        }
        let path = PathBuf::from(OsString::from_wide(path));

        let expected = if self.validating() {
            let input = String::from_utf16_lossy(&text_of(self.expected));
            let hash = input.trim();
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                self.set_status("Enter a SHA-256 hash with 64 hexadecimal characters.", Status::Neutral);
                return;
            }
            Some(hash.to_owned())
        } else {
            None
        };

        let (sender, result) = mpsc::channel();
        if unsafe { SetTimer(parent, TIMER, 40, None) } == 0 {
            self.set_status("Could not start hashing.", Status::Neutral);
            return;
        }
        if let Err(error) = thread::Builder::new().spawn(move || {
            let _ = sender.send(hash_file(path));
        }) {
            unsafe { KillTimer(parent, TIMER) };
            self.set_status(&format!("Could not start hashing. {error}"), Status::Neutral);
            return;
        }
        self.job.replace(Some(Job { expected, result }));
        self.set_busy(true);
        self.set_status("Calculating SHA-256…", Status::Neutral);
    }

    fn finish(&self, parent: HWND) {
        let result = match self.job.borrow().as_ref().map(|job| job.result.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(TryRecvError::Disconnected)) => Err(io::Error::other("Hashing was interrupted.")),
            _ => return,
        };
        let expected = self.job.take().and_then(|job| job.expected);
        unsafe { KillTimer(parent, TIMER) };
        self.set_busy(false);
        match (result, expected) {
            (Ok(hash), Some(expected)) => {
                let (text, kind) = if hash.eq_ignore_ascii_case(&expected) {
                    ("Hash matches", Status::Match)
                } else {
                    ("Hash doesn't match", Status::Mismatch)
                };
                self.set_status(text, kind);
            }
            (Ok(hash), None) => {
                set_text(self.digest, &hash);
                self.set_status("SHA-256 hash calculated.", Status::Neutral);
                unsafe { EnableWindow(self.copy, 1) };
            }
            (Err(error), _) => self.set_status(&format!("Could not read the file. {error}"), Status::Neutral),
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Window;
        match message {
            WM_CREATE => match Window::new(hwnd) {
                Ok(window) => {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(Box::new(window)) as isize);
                    DragAcceptFiles(hwnd, 1);
                    return 0;
                }
                Err(_) => return -1,
            },
            WM_DESTROY => {
                PostQuitMessage(0);
                return 0;
            }
            WM_NCDESTROY if !state.is_null() => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(state.cast_mut()));
            }
            _ => {
                if let Some(window) = state.as_ref() {
                    match message {
                        WM_ERASEBKGND => return 1,
                        WM_PAINT => window.appearance.paint_window(hwnd, window.validating()),
                        WM_PRINTCLIENT => {
                            let mut bounds = RECT::default();
                            GetClientRect(hwnd, &mut bounds);
                            window.appearance.paint_background(wparam as HDC, &bounds, window.validating());
                        }
                        WM_DRAWITEM if lparam != 0 => {
                            window.paint_item(&*(lparam as *const DRAWITEMSTRUCT));
                            return 1;
                        }
                        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
                            let control = lparam as HWND;
                            let field = control == window.path || control == window.expected || control == window.digest;
                            return window.appearance.color_control(
                                wparam as HDC, field, control == window.title || control == window.hint,
                                control == window.hint,
                            );
                        }
                        WM_COMMAND => {
                            let source = lparam as HWND;
                            let notification = (wparam >> 16) as u32;
                            if notification == EN_CHANGE && (source == window.path || source == window.expected) {
                                window.clear_result();
                            } else if notification == BN_CLICKED as u32 {
                                match wparam & 0xffff {
                                    BROWSE => window.browse(hwnd),
                                    ACTION => window.start(hwnd),
                                    CHECKER_TAB => window.select_tab(hwnd, false),
                                    VALIDATOR_TAB => window.select_tab(hwnd, true),
                                    THEME_TOGGLE => window.toggle_theme(hwnd),
                                    COPY => {
                                        SetFocus(window.digest);
                                        SendMessageW(window.digest, EM_SETSEL, 0, -1);
                                        SendMessageW(window.digest, WM_COPY, 0, 0);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        WM_DROPFILES => window.drop_file(hwnd, wparam as HDROP),
                        WM_TIMER if wparam == TIMER => window.finish(hwnd),
                        _ => return DefWindowProcW(hwnd, message, wparam, lparam),
                    }
                    return 0;
                }
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

fn create_window() -> io::Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }

        let style = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let mut bounds = RECT {
            left: 0,
            top: 0,
            right: WIDTH,
            bottom: HEIGHT,
        };
        if AdjustWindowRectEx(&mut bounds, style, 0, 0) == 0 {
            return Err(io::Error::last_os_error());
        }
        let hwnd = CreateWindowExW(
            0,
            CLASS,
            TITLE,
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return Err(io::Error::other("Could not create the application window."));
        }
        Ok(hwnd)
    }
}

fn run() -> io::Result<()> {
    let hwnd = create_window()?;
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        let mut message = MSG::default();
        loop {
            match GetMessageW(&mut message, null_mut(), 0, 0) {
                -1 => return Err(io::Error::last_os_error()),
                0 => return Ok(()),
                _ => {}
            }
            if message.message == WM_KEYDOWN {
                let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Window;
                if let Some(window) = state.as_ref() {
                    let key = message.wParam as u16;
                    if key == VK_A
                        && message.hwnd == window.expected
                        && GetKeyState(VK_CONTROL as i32) < 0
                        && GetKeyState(VK_MENU as i32) >= 0
                    {
                        SendMessageW(window.expected, EM_SETSEL, 0, -1);
                        continue;
                    }
                    if key == VK_RETURN && (message.hwnd == window.path || message.hwnd == window.expected) {
                        window.start(hwnd);
                        continue;
                    }
                    if (key == VK_LEFT || key == VK_RIGHT)
                        && (message.hwnd == window.checker_tab || message.hwnd == window.validator_tab)
                    {
                        let validating = key == VK_RIGHT;
                        window.select_tab(hwnd, validating);
                        SetFocus(if validating { window.validator_tab } else { window.checker_tab });
                        continue;
                    }
                }
            }
            if IsDialogMessageW(hwnd, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

fn main() {
    if let Err(error) = run() {
        unsafe { MessageBoxW(null_mut(), wide(&error.to_string()).as_ptr(), TITLE, MB_ICONERROR) };
    }
}
