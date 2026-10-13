//! Keyboard target dispatch preserves X11 WM focus handling as well as Wayland events.
use crate::compositor::SingleWindow;
use smithay::{
    backend::input::KeyState,
    desktop::PopupKind,
    input::{
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        Seat,
    },
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
};
use std::borrow::Cow;
#[derive(Clone, Debug, PartialEq)]
pub enum KeyboardFocus {
    Wayland(WlSurface),
    #[cfg(feature = "x11")]
    X11(smithay::xwayland::X11Surface),
}
impl From<WlSurface> for KeyboardFocus {
    fn from(surface: WlSurface) -> Self {
        Self::Wayland(surface)
    }
}
impl From<PopupKind> for KeyboardFocus {
    fn from(popup: PopupKind) -> Self {
        Self::Wayland(popup.wl_surface().clone())
    }
}
impl From<KeyboardFocus> for WlSurface {
    fn from(focus: KeyboardFocus) -> Self {
        focus
            .wl_surface()
            .expect("focused popup must have an associated surface")
            .into_owned()
    }
}
impl IsAlive for KeyboardFocus {
    fn alive(&self) -> bool {
        match self {
            Self::Wayland(surface) => surface.is_alive(),
            #[cfg(feature = "x11")]
            Self::X11(surface) => surface.alive(),
        }
    }
}
impl WaylandFocus for KeyboardFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Wayland(surface) => Some(Cow::Borrowed(surface)),
            #[cfg(feature = "x11")]
            Self::X11(surface) => surface.wl_surface().map(Cow::Owned),
        }
    }
}
impl KeyboardTarget<SingleWindow> for KeyboardFocus {
    fn enter(
        &self,
        seat: &Seat<SingleWindow>,
        state: &mut SingleWindow,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        match self {
            Self::Wayland(surface) => surface.enter(seat, state, keys, serial),
            #[cfg(feature = "x11")]
            Self::X11(surface) => surface.enter(seat, state, keys, serial),
        }
    }
    fn leave(&self, seat: &Seat<SingleWindow>, state: &mut SingleWindow, serial: Serial) {
        match self {
            Self::Wayland(surface) => surface.leave(seat, state, serial),
            #[cfg(feature = "x11")]
            Self::X11(surface) => surface.leave(seat, state, serial),
        }
    }
    fn key(
        &self,
        seat: &Seat<SingleWindow>,
        state: &mut SingleWindow,
        key: KeysymHandle<'_>,
        pressed: KeyState,
        serial: Serial,
        time: u32,
    ) {
        match self {
            Self::Wayland(surface) => {
                KeyboardTarget::key(surface, seat, state, key, pressed, serial, time)
            }
            #[cfg(feature = "x11")]
            Self::X11(surface) => {
                KeyboardTarget::key(surface, seat, state, key, pressed, serial, time)
            }
        }
    }
    fn modifiers(
        &self,
        seat: &Seat<SingleWindow>,
        state: &mut SingleWindow,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        match self {
            Self::Wayland(surface) => {
                KeyboardTarget::modifiers(surface, seat, state, modifiers, serial)
            }
            #[cfg(feature = "x11")]
            Self::X11(surface) => {
                KeyboardTarget::modifiers(surface, seat, state, modifiers, serial)
            }
        }
    }
}
