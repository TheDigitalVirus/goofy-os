use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use pc_keyboard::{DecodedKey, Error, HandleControl, KeyEvent, Keyboard, ScancodeSet1, layouts};

use crate::serial_println;

// Variáveis atômicas para modificar teclas
pub static CTRL: AtomicBool = AtomicBool::new(false);
pub static SHIFT: AtomicBool = AtomicBool::new(false);
pub static ALT: AtomicBool = AtomicBool::new(false);

// Controle de layout do teclado (0 = azerty, 1 = qwerty, 2 = dvorak)
pub static CURRENT_LAYOUT: AtomicUsize = AtomicUsize::new(0);
pub enum KeyboardLayout {
    Azerty(Keyboard<layouts::Azerty, ScancodeSet1>),
    Dvorak(Keyboard<layouts::Dvorak104Key, ScancodeSet1>),
    Qwerty(Keyboard<layouts::Us104Key, ScancodeSet1>),
}

impl KeyboardLayout {
    pub fn add_byte(&mut self, scancode: u8) -> Result<Option<KeyEvent>, Error> {
        match self {
            KeyboardLayout::Azerty(kb) => kb.add_byte(scancode),
            KeyboardLayout::Dvorak(kb) => kb.add_byte(scancode),
            KeyboardLayout::Qwerty(kb) => kb.add_byte(scancode),
        }
    }

    pub fn process_keyevent(&mut self, event: KeyEvent) -> Option<DecodedKey> {
        match self {
            KeyboardLayout::Azerty(kb) => kb.process_keyevent(event),
            KeyboardLayout::Dvorak(kb) => kb.process_keyevent(event),
            KeyboardLayout::Qwerty(kb) => kb.process_keyevent(event),
        }
    }
}

// Função para criar teclado com layout específico
pub fn create_keyboard(layout_idx: usize) -> KeyboardLayout {
    match layout_idx {
        1 => KeyboardLayout::Qwerty(Keyboard::new(ScancodeSet1::new(), layouts::Us104Key, HandleControl::MapLettersToUnicode)), // Qwerty
        2 => KeyboardLayout::Dvorak(Keyboard::new(ScancodeSet1::new(), layouts::Dvorak104Key, HandleControl::MapLettersToUnicode)), // Dvorak
        _ => KeyboardLayout::Azerty(Keyboard::new(ScancodeSet1::new(), layouts::Azerty, HandleControl::MapLettersToUnicode)), // Azerty (padrão)
    }
}

// Função para mudar layout do teclado
pub fn set_keyboard_layout(layout: &str) -> bool {
    let layout_idx = match layout {
        "azerty" => 0,
        "qwerty" => 1,
        "dvorak" => 2,
        _ => {
            serial_println!("[KEYBOARD] Layout inválido: {}", layout);
            return false;
        }
    };
    
    CURRENT_LAYOUT.store(layout_idx, Ordering::Relaxed);
    serial_println!("[KEYBOARD] Layout alterado para: {}", layout);
    true
}

// Função para obter layout atual
pub fn get_current_layout() -> &'static str {
    match CURRENT_LAYOUT.load(Ordering::Relaxed) {
        1 => "qwerty",
        2 => "dvorak",
        _ => "azerty",
    }
}

// Função para alternar entre layouts (ciclicamente)
pub fn cycle_keyboard_layout() -> bool {
    let current = CURRENT_LAYOUT.load(Ordering::Relaxed);
    let next = (current + 1) % 3;
    
    let layouts = ["azerty", "qwerty", "dvorak"];
    set_keyboard_layout(layouts[next])
}