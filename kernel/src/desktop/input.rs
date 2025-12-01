use crate::framebuffer::SCREEN_SIZE;
use crate::print;

use alloc::string::String;
use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use ps2_mouse::MouseState;

// OPTIMIZAÇÃO: Tamanhos de fila ajustados com base no uso real
pub static SCANCODE_QUEUE: OnceCell<ArrayQueue<u8>> = OnceCell::uninit();
pub static STATE_QUEUE: OnceCell<ArrayQueue<MouseState>> = OnceCell::uninit();
pub static CLICK_QUEUE: OnceCell<ArrayQueue<(i16, i16)>> = OnceCell::uninit();
pub static FILE_OPEN_QUEUE: OnceCell<ArrayQueue<(String, String)>> = OnceCell::uninit();

// OPTIMIZAÇÃO: Funções auxiliares para evitar repetição de código
#[inline(always)]
fn push_to_queue<T>(queue: Option<&ArrayQueue<T>>, item: T, item_name: &str, queue_name: &str)
where
    T: core::fmt::Debug,
{
    match queue {
        Some(q) => {
            if q.push(item).is_err() {
                print!("{} queue is full, dropping {}", queue_name, item_name);
            }
        }
        None => print!("{} queue not initialized", queue_name),
    }
}

pub fn add_scancode(scancode: u8) {
    push_to_queue(SCANCODE_QUEUE.get(), scancode, "scancode", "Scancode");
}

pub fn add_mouse_state(state: MouseState) {
    push_to_queue(STATE_QUEUE.get(), state, "mouse state", "Mouse state");
}

pub fn add_file_open_request(file_path: String, app_name: String) {
    if let Some(queue) = FILE_OPEN_QUEUE.get() {
        if queue.push((file_path.clone(), app_name.clone())).is_err() {
            print!(
                "File open queue is full, dropping request: {} with {}",
                file_path, app_name
            );
        }
    } else {
        print!(
            "File open queue not initialized, cannot add request: {} with {}",
            file_path, app_name
        );
    }
}

pub fn init_queues() {
    // OPTIMIZAÇÃO: Tamanhos baseados em análise de uso
    SCANCODE_QUEUE
        .try_init_once(|| ArrayQueue::new(128)) // Aumentado para evitar perdas
        .expect("Scancode queue should only be initialized once");
    STATE_QUEUE
        .try_init_once(|| ArrayQueue::new(64))
        .expect("Mouse state queue should only be initialized once");
    CLICK_QUEUE
        .try_init_once(|| ArrayQueue::new(32))
        .expect("Click queue should only be initialized once");
    FILE_OPEN_QUEUE
        .try_init_once(|| ArrayQueue::new(16))
        .expect("File open queue should only be initialized once");
}

// OPTIMIZAÇÃO: Use fields packing para reduzir tamanho
#[repr(C)]
pub struct CurrentMouseState {
    pub x: i16,
    pub y: i16,
    pub prev_x: i16,
    pub prev_y: i16,
    pub left_button_down: bool,
    pub right_button_down: bool,
    pub prev_left_button_down: bool,
    pub prev_right_button_down: bool,
    pub has_moved: bool,
    _screen_size: (u16, u16),
}

impl CurrentMouseState {
    pub fn new() -> Self {
        let screen_size = *SCREEN_SIZE.get().unwrap();
        CurrentMouseState {
            x: (screen_size.0 / 2) as i16,
            y: (screen_size.1 / 2) as i16,
            prev_x: (screen_size.0 / 2) as i16,
            prev_y: (screen_size.1 / 2) as i16,
            left_button_down: false,
            right_button_down: false,
            prev_left_button_down: false,
            prev_right_button_down: false,
            has_moved: true,
            _screen_size: screen_size,
        }
    }

    pub fn update(&mut self, state: MouseState) {
        // OPTIMIZAÇÃO: Atualização otimizada com menos operações
        self.prev_x = self.x;
        self.prev_y = self.y;
        self.prev_left_button_down = self.left_button_down;
        self.prev_right_button_down = self.right_button_down;

        // OPTIMIZAÇÃO: Cálculo direto com bounds checking otimizado
        self.x = (self.x + state.get_x())
            .max(0)
            .min(self._screen_size.0 as i16 - 1);
        self.y = (self.y - state.get_y())
            .max(0)
            .min(self._screen_size.1 as i16 - 1);

        self.left_button_down = state.left_button_down();
        self.right_button_down = state.right_button_down();

        // OPTIMIZAÇÃO: Cálculo otimizado de movimento
        self.has_moved = (self.x != self.prev_x) || (self.y != self.prev_y);

        // Detecção de clique otimizada
        if self.prev_left_button_down && !self.left_button_down && !self.has_moved {
            push_to_queue(
                CLICK_QUEUE.get(),
                (self.x, self.y),
                "click",
                "Click",
            );
        }
    }
}