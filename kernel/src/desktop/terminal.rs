use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use alloc::vec;
use noto_sans_mono_bitmap::{FontWeight, RasterHeight};
use pc_keyboard::KeyCode;
use crate::{
    desktop::{application::Application, keyboard::{get_current_layout, set_keyboard_layout}}, framebuffer::Color, fs::manager::list_directory, surface::{Shape, Surface}, time::{get_ms_since_epoch, get_utc_time}
    
};

#[derive(Clone, PartialEq)]
pub enum TerminalMode {
    Normal,
    Command,
}

pub struct Terminal {
    // Campos otimizados com capacidades pré-definidas
    prompt: &'static str,
    command_history: Vec<String>,
    current_command: String,
    command_cursor: usize,
    output_lines: Vec<String>,
    
    // Sistema de display do Notepad
    display_lines: Vec<String>,
    command_lines: Vec<String>,
    scroll_offset: usize,
    text_area_idx: usize,
    cursor_idx: usize,
    max_chars_per_line: usize,
    max_visible_lines: usize,
    
    // Cache para estado de renderização
    previous_content: String,
    prev_cursor_x: usize,
    prev_cursor_y: usize,
    
    // Estado otimizado
    show_cursor: bool,
    cursor_blink_state: bool,
    cursor_blink_timer: u64,
    history_index: usize,
    completion_cache: Option<(String, Vec<String>)>,
    completion_index: usize,
    mode: TerminalMode,
    needs_redraw: bool,
    last_render_time: u64,
    content_changed: bool,
}

// Comandos estáticos para lookup O(1) - ADICIONADO NOVO COMANDO
static COMMANDS: &[&str] = &[
    "help", "clear", "echo", "ls", "cat", "date", "time", 
    "uptime", "version", "setkeyboard" // Novo comando
];


// Constantes para evitar magic numbers
const COMMAND_HISTORY_CAPACITY: usize = 50;
const OUTPUT_LINES_CAPACITY: usize = 24;
const CURRENT_COMMAND_CAPACITY: usize = 100;
const CURSOR_BLINK_INTERVAL: u64 = 500;
const DISPLAY_LINES_CAPACITY: usize = 50;

impl Terminal {
    pub fn new(_args: Option<String>) -> Self {
        let mut terminal = Self {
            prompt: "goofy> ",
            command_history: Vec::with_capacity(COMMAND_HISTORY_CAPACITY),
            current_command: String::with_capacity(CURRENT_COMMAND_CAPACITY),
            command_cursor: 0,
            output_lines: Vec::with_capacity(OUTPUT_LINES_CAPACITY),
            
            // Sistema de display do Notepad
            display_lines: Vec::with_capacity(DISPLAY_LINES_CAPACITY),
            command_lines: Vec::new(),
            scroll_offset: 0,
            text_area_idx: 0,
            cursor_idx: 0,
            max_chars_per_line: 84,
            max_visible_lines: 24,
            
            // Cache para renderização
            previous_content: String::new(),
            prev_cursor_x: 0,
            prev_cursor_y: 0,
            
            // Estado
            show_cursor: true,
            cursor_blink_state: true,
            cursor_blink_timer: 0,
            history_index: 0,
            completion_cache: None,
            completion_index: 0,
            mode: TerminalMode::Command,
            needs_redraw: true,
            last_render_time: 0,
            content_changed: true,
        };

        // Mensagens iniciais
        terminal.output_lines.extend_from_slice(&[
            "GoofyOS Terminal v0.1.0".into(),
            "Type 'help' for available commands".into(),
            "".into()
        ]);
        
        // Inicializa o sistema de display
        terminal.update_display_lines();
        
        terminal
    }

    fn execute_command(&mut self, command: &str) {
        if !command.is_empty() {
            if self.command_history.len() >= COMMAND_HISTORY_CAPACITY {
                self.command_history.remove(0);
            }
            self.command_history.push(command.to_string());
        }
        self.history_index = self.command_history.len();

        let output = self.process_command(command);
        
        self.output_lines.extend(output);
        
        if self.output_lines.len() > OUTPUT_LINES_CAPACITY {
            let remove_count = self.output_lines.len() - OUTPUT_LINES_CAPACITY;
            self.output_lines.drain(0..remove_count);
        }

        // Atualiza display e scroll
        self.update_display_lines();
        self.update_scroll_if_needed();
        
        self.content_changed = true;
        self.needs_redraw = true;
        self.completion_cache = None;
    }

    fn process_command(&mut self, cmd: &str) -> Vec<String> {
        let mut parts = cmd.split_whitespace();
        let Some(first_part) = parts.next() else {
            return Vec::new();
        };

        match first_part {
            "help" => vec![
                "Available commands:".into(),
                "  help        - Show this help".into(),
                "  clear       - Clear terminal".into(),
                "  echo        - Print arguments".into(),
                "  ls          - List files".into(),
                "  cat         - Show file contents".into(),
                "  date        - Show current date/time".into(),
                "  time        - Show current time".into(),
                "  uptime      - Show system uptime".into(),
                "  version     - Show OS version".into(),
                "  setkeyboard - Change keyboard layout".into(),
                "".into(),
            ],
            "clear" => {
                self.output_lines.clear();
                self.display_lines.clear();
                self.command_lines.clear();
                vec!["Terminal cleared.".into()]
            }
            "echo" => {
                let rest = parts.collect::<Vec<&str>>().join(" ");
                if !rest.is_empty() { vec![rest] } else { vec!["".into()] }
            }
            "ls" => self.list_directory_command(),
            "cat" => {
                if let Some(filename) = parts.next() {
                    match crate::fs::manager::read_text_file(filename) {
                        Ok(content) => content.lines().map(String::from).collect(),
                        Err(e) => vec![format!("Error reading file: {}", e)],
                    }
                } else {
                    vec!["Usage: cat <filename>".into()]
                }
            }
            "date" => {
                let datetime = get_utc_time();
                vec![format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
                    datetime.year, datetime.month, datetime.day,
                    datetime.hours, datetime.minutes, datetime.seconds
                )]
            }
            "time" => {
                let datetime = get_utc_time();
                vec![format!(
                    "{:02}:{:02}:{:02}.{:03}",
                    datetime.hours, datetime.minutes, datetime.seconds, datetime.millis
                )]
            }
            "uptime" => {
                let uptime_seconds = get_ms_since_epoch() as u64 / 1000;
                let hours = uptime_seconds / 3600;
                let minutes = (uptime_seconds % 3600) / 60;
                let seconds = uptime_seconds % 60;
                vec![format!("Uptime: {:02}:{:02}:{:02}", hours, minutes, seconds)]
            }
            "version" => vec!["GoofyOS v0.1.0 - Built with Rust".into()],
            "setkeyboard" => self.set_keyboard_command(parts.next()), // Novo comando
            _ => vec![format!("Command not found: {}", cmd)],
        }
    }

    // No comando setkeyboard do terminal:
    fn set_keyboard_command(&mut self, layout: Option<&str>) -> Vec<String> {
        match layout {
            Some(layout_name) => {
                if set_keyboard_layout(&layout_name) {
                    vec![format!("Keyboard layout changed to: {}", layout_name)]
                } else {
                    vec![format!("Failed to change keyboard layout to: {}", layout_name)]
                }
            }
            None => {
                vec![
                    format!("Current keyboard layout: {:?}", get_current_layout()),
                    "Usage: setkeyboard <layout>".into(),
                ]
            }
        }
    }

    fn list_directory_command(&mut self) -> Vec<String> {
        match list_directory("/") {
            Ok(entries) => {
                let mut lines = Vec::with_capacity(entries.len() + 1);
                lines.push("Directory listing:".into());
                for entry in entries {
                    lines.push(format!("{}", entry.name));
                }
                lines
            }
            Err(e) => vec![format!("Error listing directory: {}", e)],
        }
    }

    fn break_line(&self, text: &str, max_len: usize) -> Vec<String> {
        let mut result = Vec::new();
        let mut remaining = text;

        while remaining.len() > max_len {
            let split_point = self.find_safe_split_point(remaining, max_len);
            let (chunk, rest) = remaining.split_at(split_point);
            result.push(chunk.to_string());
            remaining = rest.trim_start_matches(' ');
        }

        if !remaining.is_empty() {
            result.push(remaining.to_string());
        }

        result
    }

    fn find_safe_split_point(&self, text: &str, max_len: usize) -> usize {
        if text.len() <= max_len {
            return text.len();
        }

        // Tenta encontrar um espaço para quebrar naturalmente
        if let Some(last_space) = text[..max_len].rfind(' ') {
            if last_space > max_len / 2 {
                return last_space + 1;
            }
        }

        // Se não encontrar espaço, quebra no máximo
        max_len
    }

    fn update_display_lines(&mut self) {
        self.display_lines.clear();
        self.command_lines.clear();

        // Processa cada linha de output com wrap
        for line in &self.output_lines {
            if line.len() <= self.max_chars_per_line {
                self.display_lines.push(line.clone());
            } else {
                self.display_lines.extend(self.break_line(line, self.max_chars_per_line));
            }
        }

        // Processa linha de comando
        let command_line = format!("{}{}", self.prompt, self.current_command);
        self.command_lines = self.break_line(&command_line, self.max_chars_per_line);
        self.display_lines.extend_from_slice(&self.command_lines);
    }

    fn update_scroll_if_needed(&mut self) {
        if self.display_lines.is_empty() || self.command_lines.is_empty() {
            return;
        }

        // Calcula a linha do cursor baseado na posição dentro da linha de comando
        let cursor_line = self.get_cursor_line();
        
        // Ajusta scroll se cursor está fora da área visível
        if cursor_line < self.scroll_offset {
            self.scroll_offset = cursor_line;
            self.content_changed = true;
        } else if cursor_line >= self.scroll_offset + self.max_visible_lines {
            self.scroll_offset = cursor_line - self.max_visible_lines + 1;
            self.content_changed = true;
        }
    }

    fn get_cursor_line(&self) -> usize {
        if self.command_lines.is_empty() {
            return 0;
        }

        let cursor_in_command = self.prompt.len() + self.command_cursor;

        // Encontra em qual linha da command_lines o cursor está
        let mut char_count = 0;
        for (i, line) in self.command_lines.iter().enumerate() {
            if char_count + line.len() >= cursor_in_command {
                // O cursor está nesta linha
                let output_lines_count = self.display_lines.len() - self.command_lines.len();
                return output_lines_count + i;
            }
            char_count += line.len();
        }

        // Se não encontrou, retorna a última linha
        self.display_lines.len() - 1
    }

    fn get_display_text(&self) -> String {
        let start = self.scroll_offset;
        let end = (self.scroll_offset + self.max_visible_lines).min(self.display_lines.len());
        
        if start >= self.display_lines.len() {
            return String::new();
        }

        self.display_lines[start..end].join("\n")
    }

    fn get_cursor_visual_position(&self) -> (usize, usize) {
        if self.command_lines.is_empty() {
            return (5, 5);
        }

        let cursor_in_command = self.prompt.len() + self.command_cursor;

        // Encontra a linha e coluna dentro das command_lines
        let mut char_count = 0;
        let mut line_index = 0;
        let mut col_in_line = 0;

        for (i, line) in self.command_lines.iter().enumerate() {
            if char_count + line.len() >= cursor_in_command {
                line_index = i;
                col_in_line = cursor_in_command - char_count;
                break;
            }
            char_count += line.len();
        }

        // Calcula a linha absoluta
        let output_lines_count = self.display_lines.len() - self.command_lines.len();
        let absolute_line = output_lines_count + line_index;

        // Calcula a linha visível relativa ao scroll
        let visible_line = absolute_line.saturating_sub(self.scroll_offset);

        // CORREÇÃO: Usa constantes precisas para posicionamento igual ao Notepad
        let x = 3 + col_in_line * 7; // 7 pixels por caractere como no Notepad
        let y = 5 + visible_line * 18; // 18 pixels por linha como no Notepad

        (x, y)
    }

    // --- FUNÇÕES EXISTENTES DO TERMINAL ---

    fn update_completion_candidates(&mut self) {
        let current = &self.current_command;
        
        if let Some((cached_input, _)) = &self.completion_cache {
            if cached_input == current {
                return;
            }
        }

        let candidates: Vec<String> = if current.is_empty() {
            COMMANDS.iter().map(|&s| s.into()).collect()
        } else {
            COMMANDS
                .iter()
                .filter(|&&cmd| cmd.starts_with(current))
                .map(|&s| s.into())
                .collect()
        };
        
        self.completion_cache = Some((current.clone(), candidates));
        self.completion_index = 0;
    }

    fn complete_command(&mut self) {
        self.update_completion_candidates();
        
        if let Some((_, candidates)) = &self.completion_cache {
            if let Some(completion) = candidates.get(self.completion_index) {
                self.current_command = completion.clone();
                self.command_cursor = self.current_command.len();
                self.completion_index = (self.completion_index + 1) % candidates.len();
                self.update_display_lines();
                self.content_changed = true;
                self.needs_redraw = true;
            }
        }
    }

    fn navigate_history(&mut self, direction: isize) {
        if self.command_history.is_empty() {
            return;
        }

        let new_index = match direction {
            1 if self.history_index > 0 => self.history_index - 1,
            -1 if self.history_index < self.command_history.len() => self.history_index + 1,
            _ => return,
        };

        self.history_index = new_index;
        self.current_command = if new_index < self.command_history.len() {
            self.command_history[new_index].clone()
        } else {
            String::new()
        };
        self.command_cursor = self.current_command.len();
        self.update_display_lines();
        self.content_changed = true;
        self.needs_redraw = true;
    }

    fn update_cursor_blink(&mut self, current_time: u64) {
        if current_time - self.cursor_blink_timer > CURSOR_BLINK_INTERVAL {
            self.cursor_blink_state = !self.cursor_blink_state;
            self.cursor_blink_timer = current_time;
            self.content_changed = true;
            self.needs_redraw = true;
        }
    }

    fn scroll_lines(&mut self, lines: isize) {
        let total_lines = self.display_lines.len();
        if total_lines <= self.max_visible_lines {
            return;
        }

        let max_offset = total_lines.saturating_sub(self.max_visible_lines);
        let new_offset = match lines {
            positive if positive > 0 => {
                self.scroll_offset.saturating_add(positive as usize).min(max_offset)
            }
            negative if negative < 0 => {
                self.scroll_offset.saturating_sub(negative.abs() as usize)
            }
            _ => self.scroll_offset,
        };

        if new_offset != self.scroll_offset {
            self.scroll_offset = new_offset;
            self.content_changed = true;
            self.needs_redraw = true;
        }
    }
}

impl Application for Terminal {
    fn init(&mut self, surface: &mut Surface) {
        self.text_area_idx = surface.add_shape(Shape::Text {
            x: 5,
            y: 5,
            content: self.get_display_text(),
            color: Color::WHITE,
            background_color: Color::BLACK,
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Cursor visual como retângulo vertical (estilo Notepad)
        self.cursor_idx = surface.add_shape(Shape::Rectangle {
            x: 5,
            y: 5,
            width: 1, // Largura 1 pixel como no Notepad
            height: 16,
            color: Color::WHITE,
            filled: true,
            hide: false,
        });

        self.needs_redraw = true;
        self.content_changed = true;
    }

    fn handle_char_input(&mut self, ch: char, ctrl_pressed: bool, _surface: &mut Surface) {
        if self.mode != TerminalMode::Command {
            return;
        }

        if ctrl_pressed {
            match ch {
                'l' | 'L' => {
                    self.output_lines.clear();
                    self.display_lines.clear();
                    self.command_lines.clear();
                    self.output_lines.push("Terminal cleared".into());
                    self.output_lines.push("".into());
                    self.update_display_lines();
                    self.content_changed = true;
                    self.needs_redraw = true;
                }
                'c' | 'C' => {
                    self.current_command.clear();
                    self.command_cursor = 0;
                    self.output_lines.push("^C".into());
                    self.update_display_lines();
                    self.content_changed = true;
                    self.needs_redraw = true;
                }
                _ => {}
            }
            return;
        }

        match ch {
            '\r' | '\n' => {
                let command = core::mem::take(&mut self.current_command);
                if !command.is_empty() {
                    self.output_lines.push(format!("{}{}", self.prompt, command));
                }
                
                self.execute_command(&command);
                self.current_command.clear();
                self.command_cursor = 0;
            }
            '\x08' => { // Backspace
                if self.command_cursor > 0 {
                    self.current_command.remove(self.command_cursor - 1);
                    self.command_cursor -= 1;
                    self.update_display_lines();
                    self.content_changed = true;
                    self.needs_redraw = true;
                }
            }
            '\t' => self.complete_command(),
            ch if !ch.is_control() => {
                // Garante que o command_cursor está dentro dos limites
                if self.command_cursor <= self.current_command.len() {
                    self.current_command.insert(self.command_cursor, ch);
                    self.command_cursor += 1;
                    self.update_display_lines();
                    self.content_changed = true;
                    self.needs_redraw = true;
                }
            }
            _ => {}
        }
        
        self.update_scroll_if_needed();
    }

    fn handle_key_input(&mut self, key: KeyCode, _surface: &mut Surface) {
        if self.mode != TerminalMode::Command {
            return;
        }

        match key {
            KeyCode::ArrowLeft if self.command_cursor > 0 => {
                self.command_cursor -= 1;
                self.content_changed = true;
                self.needs_redraw = true;
            }
            KeyCode::ArrowRight if self.command_cursor < self.current_command.len() => {
                self.command_cursor += 1;
                self.content_changed = true;
                self.needs_redraw = true;
            }
            KeyCode::ArrowUp => {
                if self.current_command.is_empty() {
                    self.scroll_lines(-1); // Scroll para cima
                } else {
                    self.navigate_history(1); // Navegação no histórico
                }
            }
            KeyCode::ArrowDown => {
                if self.current_command.is_empty() {
                    self.scroll_lines(1); // Scroll para baixo
                } else {
                    self.navigate_history(-1); // Navegação no histórico
                }
            }
            KeyCode::Home => {
                self.command_cursor = 0;
                self.content_changed = true;
                self.needs_redraw = true;
            }
            KeyCode::End => {
                self.command_cursor = self.current_command.len();
                self.content_changed = true;
                self.needs_redraw = true;
            }
            KeyCode::Delete if self.command_cursor < self.current_command.len() => {
                self.current_command.remove(self.command_cursor);
                self.update_display_lines();
                self.content_changed = true;
                self.needs_redraw = true;
            }
            KeyCode::PageUp => {
                self.scroll_lines(-(self.max_visible_lines as isize) / 2);
            }
            KeyCode::PageDown => {
                self.scroll_lines((self.max_visible_lines as isize) / 2);
            }
            _ => return,
        }
        
        self.update_scroll_if_needed();
    }

    fn render(&mut self, surface: &mut Surface) {
        let current_time = get_ms_since_epoch() as u64;
        self.update_cursor_blink(current_time);

        if !self.needs_redraw && !self.content_changed {
            return;
        }

        // Atualiza conteúdo do texto se necessário
        let current_display = self.get_display_text();
        if self.content_changed || current_display != self.previous_content {
            surface.update_text_content(self.text_area_idx, current_display.clone(), None);
            self.previous_content = current_display;
            self.content_changed = false;
        }

        // Atualiza posição do cursor
        let (cursor_x, cursor_y) = self.get_cursor_visual_position();
        if cursor_x != self.prev_cursor_x || cursor_y != self.prev_cursor_y {
            surface.move_shape(self.cursor_idx, cursor_x, cursor_y);
            self.prev_cursor_x = cursor_x;
            self.prev_cursor_y = cursor_y;
        }

        // Controla visibilidade do cursor baseado no blink
        surface.update_shape_visibility(self.cursor_idx, self.show_cursor && self.cursor_blink_state);

        self.needs_redraw = false;
        self.last_render_time = current_time;
    }

    fn handle_mouse_click(&mut self, _x: usize, _y: usize, _surface: &mut Surface) {
        // Funcionalidade preservada
    }

    fn get_title(&self) -> Option<String> {
        Some("Terminal".into())
    }
}