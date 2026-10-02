//! Focused seed editing, independent of gameplay key bindings.
use super::{Action, Menu, Page};
use bevy::{
    input::{keyboard::KeyboardInput, ButtonState},
    prelude::*,
};

pub struct Editor {
    text: String,
    cursor: usize,
    selected: bool,
}
impl Editor {
    pub fn new(text: String) -> Self {
        let cursor = text.chars().count();
        Self {
            text,
            cursor,
            selected: true,
        }
    }
    fn byte(&self, cursor: usize) -> usize {
        self.text
            .char_indices()
            .nth(cursor)
            .map_or(self.text.len(), |(i, _)| i)
    }
    fn insert(&mut self, text: &str) {
        if self.selected {
            self.text.clear();
            self.cursor = 0;
            self.selected = false;
        }
        let added: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(128 - self.text.chars().count())
            .collect();
        let at = self.byte(self.cursor);
        self.text.insert_str(at, &added);
        self.cursor += added.chars().count();
    }
    fn delete(&mut self, backward: bool) {
        if self.selected {
            self.text.clear();
            self.cursor = 0;
            self.selected = false;
            return;
        }
        let start = if backward {
            self.cursor.saturating_sub(1)
        } else {
            self.cursor
        };
        let end = if backward {
            self.cursor
        } else {
            (self.cursor + 1).min(self.text.chars().count())
        };
        self.text
            .replace_range(self.byte(start)..self.byte(end), "");
        self.cursor = start;
    }
    fn move_cursor(&mut self, key: KeyCode) {
        let end = self.text.chars().count();
        self.cursor = match key {
            KeyCode::Home => 0,
            KeyCode::End => end,
            KeyCode::ArrowLeft => {
                if self.selected {
                    0
                } else {
                    self.cursor.saturating_sub(1)
                }
            }
            _ => {
                if self.selected {
                    end
                } else {
                    (self.cursor + 1).min(end)
                }
            }
        };
        self.selected = false;
    }
    pub fn display(&self) -> String {
        let start = self.cursor.saturating_sub(30);
        let mut value: String = self.text.chars().skip(start).take(36).collect();
        if !self.selected {
            let at = value
                .char_indices()
                .nth(self.cursor - start)
                .map_or(value.len(), |(i, _)| i);
            value.insert(at, '|');
        }
        if value.is_empty() {
            value = "|  Random seed".into();
        }
        if start > 0 {
            value.insert(0, '…');
        }
        value
    }
}
pub(super) fn random_seed() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}
fn resolve(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.parse().unwrap_or_else(|_| {
        text.encode_utf16()
            .fold(0i32, |hash, c| hash.wrapping_mul(31).wrapping_add(c as i32)) as i64
    }))
}
pub(super) fn commit(menu: &mut Menu) {
    if let Some(edit) = menu.seed_edit.take() {
        menu.world_seed = resolve(&edit.text).unwrap_or_else(random_seed);
        menu.status = format!("World seed: {}", menu.world_seed);
        menu.dirty = true;
    }
}
pub(crate) fn input(
    mut menu: ResMut<Menu>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut events: MessageReader<KeyboardInput>,
    actions: Query<(&Interaction, &Action), Changed<Interaction>>,
    enabled: Res<crate::player::InputEnabled>,
) {
    let events: Vec<_> = events.read().cloned().collect();
    if !enabled.0 || menu.seed_edit.is_none() {
        return;
    }
    if menu.page != Some(Page::Play) {
        menu.seed_edit = None;
        return;
    }
    let control = (keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight))
        && !keys.pressed(KeyCode::AltLeft)
        && !keys.pressed(KeyCode::AltRight);
    for event in events
        .into_iter()
        .filter(|e| e.state == ButtonState::Pressed)
    {
        let Some(editor) = menu.seed_edit.as_mut() else {
            break;
        };
        match event.key_code {
            KeyCode::Escape => {
                menu.seed_edit = None;
                menu.dirty = true;
                break;
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Tab => {
                commit(&mut menu);
                break;
            }
            KeyCode::KeyA if control => editor.selected = true,
            KeyCode::KeyV if control => match clipboard_text() {
                Ok(text) => editor.insert(&text),
                Err(message) => menu.status = message.into(),
            },
            KeyCode::Backspace => editor.delete(true),
            KeyCode::Delete => editor.delete(false),
            KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Home | KeyCode::End => {
                editor.move_cursor(event.key_code)
            }
            _ if !control => {
                if let Some(text) = event.text {
                    editor.insert(&text);
                }
            }
            _ => {}
        }
        menu.dirty = true;
    }
    // Keep held modifiers, but consume shortcuts such as O and Escape before menu input.
    keys.clear();
    if menu.seed_edit.is_some() {
        let pressed: Vec<_> = actions
            .iter()
            .filter(|(i, _)| **i == Interaction::Pressed)
            .map(|(_, a)| a)
            .collect();
        if pressed.iter().any(|a| matches!(a, Action::NewSeed)) {
            menu.seed_edit = None;
        } else if pressed.iter().any(|a| !matches!(a, Action::EditSeed))
            || (mouse.just_pressed(MouseButton::Left) && pressed.is_empty())
        {
            commit(&mut menu);
        }
    }
}

// Only called on the player's explicit Ctrl+V; never read the clipboard on focus.
#[cfg(windows)]
fn clipboard_text() -> Result<String, &'static str> {
    // Unicode clipboard handles are owned by Windows; only lock them while copying.
    // Keep this tiny platform bridge dependency-free so enabling paste does not
    // change the renderer's dependency feature set.
    use std::ffi::c_void;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn GetClipboardData(format: u32) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalLock(handle: *mut c_void) -> *mut c_void;
        fn GlobalSize(handle: *mut c_void) -> usize;
        fn GlobalUnlock(handle: *mut c_void) -> i32;
    }
    struct Close;
    impl Drop for Close {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err("Clipboard busy. Try Ctrl+V again.");
        }
        let _close = Close;
        let handle = GetClipboardData(13); // CF_UNICODETEXT
        if handle.is_null() {
            return Err("The clipboard does not contain text.");
        }
        let count = (GlobalSize(handle) / 2).min(4096);
        let pointer = GlobalLock(handle).cast::<u16>();
        if pointer.is_null() {
            return Err("Could not read clipboard text.");
        }
        let chars = std::slice::from_raw_parts(pointer, count);
        let end = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
        let text = String::from_utf16_lossy(&chars[..end]);
        GlobalUnlock(handle);
        Ok(text)
    }
}
#[cfg(not(windows))]
fn clipboard_text() -> Result<String, &'static str> {
    Err("Paste is currently available on Windows. Type the seed instead.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clicking_go_commits_pending_seed_before_launch() {
        let mut app=App::new();
        app.init_resource::<Menu>().init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>().add_message::<KeyboardInput>()
            .insert_resource(crate::player::InputEnabled(true)).add_systems(Update,input);
        {let mut menu=app.world_mut().resource_mut::<Menu>();menu.page=Some(Page::Play);menu.seed_edit=Some(Editor::new("hello".into()));}
        app.world_mut().spawn((Interaction::Pressed,Action::Start));app.update();
        let menu=app.world().resource::<Menu>();assert_eq!(menu.world_seed,99162322);assert!(menu.seed_edit.is_none());
    }
    #[test]
    fn editing_replaces_selection_and_handles_signed_limits_unicode_and_cursor() {
        let mut edit = Editor::new("42".into());
        edit.insert("-9223372036854775808");
        assert_eq!(resolve(&edit.text), Some(i64::MIN));
        edit.selected = true;
        edit.insert("9223372036854775807");
        assert_eq!(resolve(&edit.text), Some(i64::MAX));
        edit.selected = true;
        edit.insert("123");
        edit.move_cursor(KeyCode::ArrowLeft);
        edit.insert("9");
        assert_eq!(edit.text, "1293");
        edit.delete(true);
        edit.delete(false);
        assert_eq!(edit.text, "12");
        edit.selected = true;
        edit.insert("a😀b");
        edit.move_cursor(KeyCode::ArrowLeft);
        edit.delete(true);
        assert_eq!(edit.text, "ab");
        assert_eq!(resolve("hello"), Some(99162322));
        assert_eq!(resolve("  -42 \r\n"), Some(-42));
        assert_eq!(resolve(" "), None);
        edit.selected = true;
        edit.insert(&"x".repeat(500));
        assert_eq!(edit.text.chars().count(), 128);
    }
}
