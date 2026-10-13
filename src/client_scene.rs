//! Validate independent window lifecycles before the native UI consumes network data.
use crate::{Decoder, Dimensions, Message, MAX_SCENE_PIXELS, MAX_WINDOWS};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
pub struct ClientSurface {
    pub title: String,
    pub parent: Option<u32>,
    pub advertised: Dimensions,
    pub decoder: Decoder,
    pub ready: bool,
}
#[derive(Default)]
pub struct ClientScene {
    pub windows: BTreeMap<u32, ClientSurface>,
    pub closing: BTreeSet<u32>,
    pub multi: bool,
}
fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text)
}
impl ClientScene {
    /// Returns the changed window id; lifecycle removal is reflected in the map.
    pub fn apply(&mut self, message: Message) -> io::Result<Option<u32>> {
        match message {
            Message::WindowInfo {
                id,
                parent,
                size,
                title,
            } => {
                size.validate()?;
                if !self.multi && !self.windows.is_empty() {
                    return Err(invalid("mixed demo and window-scoped stream"));
                }
                if id == 0 || parent == Some(id) || parent == Some(0) {
                    return Err(invalid("invalid window identity"));
                }
                let mut ancestor = parent;
                let mut visited = BTreeSet::from([id]);
                while let Some(next) = ancestor {
                    if !visited.insert(next) {
                        return Err(invalid("cyclic window parent"));
                    }
                    ancestor = self.windows.get(&next).and_then(|window| window.parent);
                }
                let total: usize = self
                    .windows
                    .iter()
                    .filter(|(other, _)| **other != id)
                    .map(|(_, window)| window.advertised.pixel_count())
                    .sum();
                if total + size.pixel_count() > MAX_SCENE_PIXELS
                    || (!self.windows.contains_key(&id) && self.windows.len() >= MAX_WINDOWS)
                {
                    return Err(invalid("scene budget exceeded"));
                }
                self.multi = true;
                if !self.closing.contains(&id) {
                    let window = self.windows.entry(id).or_insert_with(|| ClientSurface {
                        title: String::new(),
                        parent,
                        advertised: size,
                        decoder: Decoder::default(),
                        ready: false,
                    });
                    window.title = crate::window_title(&title);
                    window.parent = parent;
                    window.advertised = size;
                }
                Ok(Some(id))
            }
            Message::WindowGone { id } => {
                self.windows.remove(&id);
                self.closing.remove(&id);
                Ok(Some(id))
            }
            Message::ForWindow { id, message } => {
                if self.closing.contains(&id) {
                    return Ok(None);
                }
                let window = self
                    .windows
                    .get_mut(&id)
                    .ok_or_else(|| invalid("frame for unknown window"))?;
                if !matches!(message.as_ref(),Message::Frame {size,..} if *size==window.advertised)
                {
                    return Err(invalid("window frame does not match metadata"));
                }
                window.decoder.apply(*message)?;
                window.ready = true;
                Ok(Some(id))
            }
            frame @ Message::Frame { .. } if !self.multi => {
                let window = self.windows.entry(1).or_insert_with(|| ClientSurface {
                    title: "Waytransfer demo".into(),
                    parent: None,
                    advertised: Dimensions::default(),
                    decoder: Decoder::default(),
                    ready: false,
                });
                window.decoder.apply(frame)?;
                window.advertised = window.decoder.size;
                window.ready = true;
                Ok(Some(1))
            }
            _ => Err(invalid("unexpected server message")),
        }
    }
    pub fn close_locally(&mut self, id: u32) {
        self.windows.remove(&id);
        self.closing.insert(id);
    }
}
