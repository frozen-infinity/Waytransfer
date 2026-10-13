//! Physical minifb keys to Linux evdev codes. The compositor currently uses a US layout.
use minifb::{Key, Window};

/// Query the actual key window on macOS. minifb 0.29's macOS `is_active`
/// inverts its native bool, and its native notification tracker also treats
/// resign-main as active. Inverting that API alone does not fix focus loss.
pub fn window_focused(window: &Window) -> bool {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn waytransfer_window_focused(window: *mut std::ffi::c_void) -> bool;
        }
        // SAFETY: minifb returns the live NSWindow pointer on macOS. The window
        // owns it for this call; viewer polling and AppKit run on the main thread.
        unsafe { waytransfer_window_focused(window.get_window_handle()) }
    }
    #[cfg(not(target_os = "macos"))]
    window.is_active()
}

pub fn evdev(key: Key) -> Option<u16> {
    Some(match key {
        Key::Escape => 1,
        Key::Key1 => 2,
        Key::Key2 => 3,
        Key::Key3 => 4,
        Key::Key4 => 5,
        Key::Key5 => 6,
        Key::Key6 => 7,
        Key::Key7 => 8,
        Key::Key8 => 9,
        Key::Key9 => 10,
        Key::Key0 => 11,
        Key::Minus => 12,
        Key::Equal => 13,
        Key::Backspace => 14,
        Key::Tab => 15,
        Key::Q => 16,
        Key::W => 17,
        Key::E => 18,
        Key::R => 19,
        Key::T => 20,
        Key::Y => 21,
        Key::U => 22,
        Key::I => 23,
        Key::O => 24,
        Key::P => 25,
        Key::LeftBracket => 26,
        Key::RightBracket => 27,
        Key::Enter => 28,
        Key::LeftCtrl => 29,
        Key::A => 30,
        Key::S => 31,
        Key::D => 32,
        Key::F => 33,
        Key::G => 34,
        Key::H => 35,
        Key::J => 36,
        Key::K => 37,
        Key::L => 38,
        Key::Semicolon => 39,
        Key::Apostrophe => 40,
        Key::Backquote => 41,
        Key::LeftShift => 42,
        Key::Backslash => 43,
        Key::Z => 44,
        Key::X => 45,
        Key::C => 46,
        Key::V => 47,
        Key::B => 48,
        Key::N => 49,
        Key::M => 50,
        Key::Comma => 51,
        Key::Period => 52,
        Key::Slash => 53,
        Key::RightShift => 54,
        Key::NumPadAsterisk => 55,
        Key::LeftAlt => 56,
        Key::Space => 57,
        Key::CapsLock => 58,
        Key::F1 => 59,
        Key::F2 => 60,
        Key::F3 => 61,
        Key::F4 => 62,
        Key::F5 => 63,
        Key::F6 => 64,
        Key::F7 => 65,
        Key::F8 => 66,
        Key::F9 => 67,
        Key::F10 => 68,
        Key::NumLock => 69,
        Key::ScrollLock => 70,
        Key::NumPad7 => 71,
        Key::NumPad8 => 72,
        Key::NumPad9 => 73,
        Key::NumPadMinus => 74,
        Key::NumPad4 => 75,
        Key::NumPad5 => 76,
        Key::NumPad6 => 77,
        Key::NumPadPlus => 78,
        Key::NumPad1 => 79,
        Key::NumPad2 => 80,
        Key::NumPad3 => 81,
        Key::NumPad0 => 82,
        Key::NumPadDot => 83,
        Key::F11 => 87,
        Key::F12 => 88,
        Key::NumPadEnter => 96,
        Key::RightCtrl => 97,
        Key::NumPadSlash => 98,
        Key::RightAlt => 100,
        Key::Home => 102,
        Key::Up => 103,
        Key::PageUp => 104,
        Key::Left => 105,
        Key::Right => 106,
        Key::End => 107,
        Key::Down => 108,
        Key::PageDown => 109,
        Key::Insert => 110,
        Key::Delete => 111,
        Key::Pause => 119,
        Key::LeftSuper => 125,
        Key::RightSuper => 126,
        Key::Menu => 127,
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn common_keys() {
        assert_eq!(evdev(Key::A), Some(30));
        assert_eq!(evdev(Key::RightCtrl), Some(97));
        assert_eq!(evdev(Key::Unknown), None);
    }
}
