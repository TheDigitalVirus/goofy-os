use alloc::string::String;
use pc_keyboard::KeyCode;

use crate::surface::Surface;

pub trait Application {
    fn init(&mut self, surface: &mut Surface);
    fn render(&mut self, surface: &mut Surface);
    fn get_title(&self) -> Option<String>;
    fn handle_char_input(&mut self, c: char, ctrl_pressed: bool, surface: &mut Surface);
    fn handle_key_input(&mut self, key: KeyCode, surface: &mut Surface);
    fn handle_mouse_click(&mut self, x: usize, y: usize, surface: &mut Surface);
}
