use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use noto_sans_mono_bitmap::{FontWeight, RasterHeight};
use pc_keyboard::KeyCode;

use crate::{
    desktop::application::Application,
    framebuffer::Color,
    fs::{
        fat32::FileEntry,
        manager::{list_directory, read_text_file, write_file},
    },
    serial_println,
    surface::{Shape, Surface},
};

#[derive(Clone, PartialEq)]
pub enum NotepadMode {
    Normal,
    SaveAs,
}

pub struct Notepad {
    text_content: String,
    cursor_position: usize,
    scroll_offset: usize,
    display_lines: Vec<String>,
    text_area_idx: usize,
    cursor_idx: usize,
    max_chars_per_line: usize,
    max_visible_lines: usize,
    previous_content: String,
    prev_cursor_x: usize,
    prev_cursor_y: usize,
    open_file_path: Option<String>,
    has_changes: bool,
    mode: NotepadMode,

    // Save-As dialog fields
    save_as_current_path: String,
    save_as_filename: String,
    save_as_folders: Vec<FileEntry>,
    save_as_selected_folder: Option<usize>,
    save_as_scroll_offset: usize,

    // Save-As dialog UI tracking for optimized rendering
    save_as_ui_initialized: bool,
    save_as_previous_path: String,
    save_as_previous_filename: String,
    save_as_previous_selected: Option<usize>,
    save_as_folder_shapes: Vec<(usize, usize)>, // (background_idx, text_idx) for each folder
    save_as_filename_text_idx: Option<usize>,
    save_as_path_text_idx: Option<usize>,
    save_as_folder_shapes_changed: bool,
}

impl Notepad {
    pub fn new(args: Option<String>) -> Self {
        let text_content = if let Some(ref path) = args {
            match read_text_file(path) {
                Ok(content) => content,
                Err(error) => {
                    serial_println!("Failed to open file {}", path);
                    serial_println!("Error: {}", error);
                    String::new()
                }
            }
        } else {
            String::new()
        };

        Self {
            text_content,
            cursor_position: 0,
            scroll_offset: 0,
            display_lines: Vec::new(),
            text_area_idx: 0,
            cursor_idx: 0,
            max_chars_per_line: 84, // Approximate characters that fit in the text area
            max_visible_lines: 22,  // Number of lines visible in the text area
            previous_content: String::new(),
            prev_cursor_x: 0,
            prev_cursor_y: 0,
            open_file_path: args,
            has_changes: false,
            mode: NotepadMode::Normal,

            // Save-As dialog fields
            save_as_current_path: "/".to_string(),
            save_as_filename: "untitled.txt".to_string(),
            save_as_folders: Vec::new(),
            save_as_selected_folder: None,
            save_as_scroll_offset: 0,

            // Save-As dialog UI tracking
            save_as_ui_initialized: false,
            save_as_previous_path: String::new(),
            save_as_previous_filename: String::new(),
            save_as_previous_selected: None,
            save_as_folder_shapes: Vec::new(),
            save_as_filename_text_idx: None,
            save_as_path_text_idx: None,
            save_as_folder_shapes_changed: false,
        }
    }

    fn handle_save(&mut self) {
        if self.open_file_path.is_none() {
            // No file path set, show Save-As dialog
            self.enter_save_as_mode();
            return;
        }

        match write_file(
            self.open_file_path.as_ref().unwrap(),
            &self.text_content.as_bytes(),
        ) {
            Ok(_) => {
                self.has_changes = false;
                self.previous_content += " "; // Trigger redraw
                serial_println!(
                    "File {} saved successfully.",
                    self.open_file_path.as_ref().unwrap()
                );
            }
            Err(e) => {
                serial_println!(
                    "Failed to save file {}: {}",
                    self.open_file_path.as_ref().unwrap(),
                    e
                );
            }
        }
    }

    fn enter_save_as_mode(&mut self) {
        self.mode = NotepadMode::SaveAs;

        // Initialize with a default filename if we don't have one
        if let Some(ref path) = self.open_file_path {
            self.save_as_filename = path.split('/').last().unwrap_or("untitled.txt").to_string();
        } else {
            self.save_as_filename = "untitled.txt".to_string();
        }

        // Start in the root directory
        self.save_as_current_path = "/".to_string();
        self.refresh_save_as_folder_list();

        // Reset UI tracking
        self.save_as_ui_initialized = false;
        self.save_as_previous_path.clear();
        self.save_as_previous_filename.clear();
        self.save_as_previous_selected = None;
        self.save_as_folder_shapes.clear();
        self.save_as_filename_text_idx = None;
        self.save_as_path_text_idx = None;
        self.save_as_folder_shapes_changed = true;
    }

    fn refresh_save_as_folder_list(&mut self) {
        match list_directory(&self.save_as_current_path) {
            Ok(entries) => {
                // Only show directories in the Save-As dialog
                self.save_as_folders = entries
                    .into_iter()
                    .filter(|e| e.is_directory && e.name != ".")
                    .collect();
                self.save_as_selected_folder = None;
                self.save_as_scroll_offset = 0;

                // Mark that we need to update the folder list UI
                self.save_as_folder_shapes_changed = true;
            }
            Err(e) => {
                serial_println!(
                    "Failed to list directory {}: {}",
                    self.save_as_current_path,
                    e
                );
                self.save_as_folders.clear();
                self.save_as_folder_shapes_changed = true;
            }
        }
    }

    fn handle_save_as_navigation(&mut self, folder_name: &str) {
        if folder_name == ".." {
            // Go up one level
            if self.save_as_current_path != "/" {
                let mut parts: Vec<&str> = self.save_as_current_path.split('/').collect();
                parts.pop(); // Remove last part
                if parts.len() <= 1 {
                    self.save_as_current_path = "/".to_string();
                } else {
                    self.save_as_current_path = parts.join("/");
                }
                self.refresh_save_as_folder_list();
            }
        } else {
            // Navigate into folder
            let new_path = if self.save_as_current_path == "/" {
                format!("/{}", folder_name)
            } else {
                format!("{}/{}", self.save_as_current_path, folder_name)
            };
            self.save_as_current_path = new_path;
            self.refresh_save_as_folder_list();
        }
    }

    fn perform_save_as(&mut self) -> bool {
        if self.save_as_filename.trim().is_empty() {
            serial_println!("Please enter a filename");
            return false;
        }

        let file_path = if self.save_as_current_path == "/" {
            format!("/{}", self.save_as_filename)
        } else {
            format!("{}/{}", self.save_as_current_path, self.save_as_filename)
        };

        match write_file(&file_path, &self.text_content.as_bytes()) {
            Ok(_) => {
                self.open_file_path = Some(file_path.clone());
                self.has_changes = false;
                self.mode = NotepadMode::Normal;
                self.previous_content += " "; // Trigger redraw
                serial_println!("File saved as: {}", file_path);
                true
            }
            Err(e) => {
                serial_println!("Failed to save file {}: {}", file_path, e);
                false
            }
        }
    }

    fn update_display_lines(&mut self) {
        self.display_lines.clear();

        // Split text into lines and wrap long lines
        let lines: Vec<&str> = self.text_content.split('\n').collect();

        for line in lines {
            if line.len() <= self.max_chars_per_line {
                self.display_lines.push(line.to_string());
            } else {
                // Wrap long lines
                let mut remaining = line;
                while remaining.len() > self.max_chars_per_line {
                    let (chunk, rest) = remaining.split_at(self.max_chars_per_line);
                    self.display_lines.push(chunk.to_string());
                    remaining = rest;
                }
                if !remaining.is_empty() {
                    self.display_lines.push(remaining.to_string());
                }
            }
        }
    }

    fn update_scroll_if_needed(&mut self) {
        // Calculate which line the cursor is on
        let mut char_count = 0;
        let mut cursor_line = 0;

        for (line_idx, line) in self.display_lines.iter().enumerate() {
            if char_count + line.len() + 1 > self.cursor_position {
                cursor_line = line_idx;
                break;
            }
            char_count += line.len() + 1; // +1 for newline
        }

        // Adjust scroll if cursor is outside visible area
        if cursor_line < self.scroll_offset {
            self.scroll_offset = cursor_line;
        } else if cursor_line >= self.scroll_offset + self.max_visible_lines {
            self.scroll_offset = cursor_line - self.max_visible_lines + 1;
        }
    }

    fn get_display_text(&self) -> String {
        let visible_lines: Vec<String> = self
            .display_lines
            .iter()
            .skip(self.scroll_offset)
            .take(self.max_visible_lines)
            .cloned()
            .collect();

        visible_lines.join("\n")
    }

    fn get_cursor_visual_position(&self) -> (usize, usize) {
        // Calculate cursor position relative to the visible text area
        let mut char_count = 0;
        let mut line_in_visible = 0;
        let mut col_in_line = 0;

        for (line_idx, line) in self.display_lines.iter().enumerate() {
            if line_idx < self.scroll_offset {
                char_count += line.len() + 1;
                continue;
            }

            if char_count + line.len() + 1 > self.cursor_position {
                col_in_line = self.cursor_position - char_count;
                break;
            }

            char_count += line.len() + 1;
            line_in_visible += 1;

            if line_in_visible >= self.max_visible_lines {
                break;
            }
        }

        // Convert to pixel coordinates (approximate)
        let x = 3 + col_in_line * 7; // 8 pixels per character (approximate)
        let y = 5 + line_in_visible * 18; // 18 pixels per line (16 + spacing)

        (x, y)
    }

    fn render_save_as_dialog(&mut self, surface: &mut Surface) {
        let width = surface.width;
        let height = surface.height;

        // Initialize dialog UI if needed
        if !self.save_as_ui_initialized {
            surface.clear_all_shapes();

            // Title
            surface.add_shape(Shape::Text {
                x: 30,
                y: 35,
                content: "Save As".to_string(),
                color: Color::BLACK,
                background_color: Color::WHITE,
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Bold,
                hide: false,
            });

            // Current path
            self.save_as_path_text_idx = Some(surface.add_shape(Shape::Text {
                x: 30,
                y: 60,
                content: format!("Location: {}", self.save_as_current_path),
                color: Color::BLACK,
                background_color: Color::WHITE,
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            }));

            // Filename input label
            let filename_y = height - 120;
            surface.add_shape(Shape::Text {
                x: 30,
                y: filename_y,
                content: "Filename:".to_string(),
                color: Color::BLACK,
                background_color: Color::WHITE,
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            // Filename input box
            surface.add_shape(Shape::Rectangle {
                x: 30,
                y: filename_y + 20,
                width: width - 80,
                height: 25,
                color: Color::new(240, 240, 240),
                filled: true,
                hide: false,
            });

            surface.add_shape(Shape::Rectangle {
                x: 30,
                y: filename_y + 20,
                width: width - 80,
                height: 25,
                color: Color::BLACK,
                filled: false,
                hide: false,
            });

            // Filename text
            self.save_as_filename_text_idx = Some(surface.add_shape(Shape::Text {
                x: 35,
                y: filename_y + 25,
                content: format!("{}_", self.save_as_filename),
                color: Color::BLACK,
                background_color: Color::new(240, 240, 240),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            }));

            // Buttons
            let button_y = height - 60;

            // Save button
            surface.add_shape(Shape::Rectangle {
                x: 30,
                y: button_y,
                width: 70,
                height: 25,
                color: Color::new(180, 255, 180),
                filled: true,
                hide: false,
            });

            surface.add_shape(Shape::Text {
                x: 50,
                y: button_y + 5,
                content: "Save".to_string(),
                color: Color::BLACK,
                background_color: Color::new(180, 255, 180),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            // Cancel button
            surface.add_shape(Shape::Rectangle {
                x: 110,
                y: button_y,
                width: 70,
                height: 25,
                color: Color::new(255, 180, 180),
                filled: true,
                hide: false,
            });

            surface.add_shape(Shape::Text {
                x: 125,
                y: button_y + 5,
                content: "Cancel".to_string(),
                color: Color::BLACK,
                background_color: Color::new(255, 180, 180),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            // Instructions
            surface.add_shape(Shape::Text {
                x: 30,
                y: height - 35,
                content:
                    "Use arrow keys to navigate, Enter to open folder, type filename and click Save"
                        .to_string(),
                color: Color::new(100, 100, 100),
                background_color: Color::WHITE,
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            self.save_as_ui_initialized = true;
        }

        // Update path if changed
        if self.save_as_current_path != self.save_as_previous_path {
            if let Some(path_idx) = self.save_as_path_text_idx {
                surface.update_text_content(
                    path_idx,
                    format!("Location: {}", self.save_as_current_path),
                    None,
                );
            }
            self.save_as_previous_path = self.save_as_current_path.clone();
        }

        // Update filename if changed
        if self.save_as_filename != self.save_as_previous_filename {
            if let Some(filename_idx) = self.save_as_filename_text_idx {
                surface.update_text_content(
                    filename_idx,
                    format!("{}_", self.save_as_filename),
                    None,
                );
            }
            self.save_as_previous_filename = self.save_as_filename.clone();
        }

        // Update folder list if needed
        if self.save_as_folder_shapes_changed {
            self.update_save_as_folder_list(surface);
        }

        // Update selection highlighting if changed
        if self.save_as_selected_folder != self.save_as_previous_selected {
            self.update_save_as_selection(surface);
            self.save_as_previous_selected = self.save_as_selected_folder;
        }
    }

    fn restore_normal_view(&mut self, surface: &mut Surface) {
        self.save_as_folder_shapes.clear();
        self.save_as_selected_folder = None;

        surface.clear_all_shapes();

        self.text_area_idx = surface.add_shape(Shape::Text {
            x: 5,
            y: 5,
            content: self.get_display_text(),
            color: Color::BLACK,
            background_color: Color::WHITE,
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        self.cursor_idx = surface.add_shape(Shape::Text {
            x: 5,
            y: 5,
            content: "|".to_string(),
            color: Color::BLACK,
            background_color: Color::WHITE,
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });
    }

    fn update_save_as_folder_list(&mut self, surface: &mut Surface) {
        // Clear old folder shapes
        let mut rm_offset = 0;
        for (bg_idx, text_idx) in &self.save_as_folder_shapes {
            surface.remove_shape(*bg_idx - rm_offset);
            surface.remove_shape(*text_idx - rm_offset - 1);
            rm_offset += 2;
        }

        self.save_as_folder_shapes_changed = false;
        self.save_as_folder_shapes.clear();

        let width = surface.width;
        let list_start_y = 85;

        let max_visible_folders = 8;

        for (i, folder) in self
            .save_as_folders
            .iter()
            .enumerate()
            .skip(self.save_as_scroll_offset)
            .take(max_visible_folders)
        {
            let y_pos = list_start_y + (i - self.save_as_scroll_offset) * 20;

            // Folder background
            let bg_idx = surface.add_shape(Shape::Rectangle {
                x: 30,
                y: y_pos,
                width: width - 80,
                height: 18,
                color: Color::WHITE,
                filled: true,
                hide: false,
            });

            // Folder name
            let text_idx = surface.add_shape(Shape::Text {
                x: 35,
                y: y_pos + 2,
                content: format!("/{}", folder.name),
                color: Color::BLACK,
                background_color: Color::WHITE,
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            self.save_as_folder_shapes.push((bg_idx, text_idx));
        }
    }

    fn update_save_as_selection(&mut self, surface: &mut Surface) {
        let max_visible_folders = 8;

        for (display_idx, (bg_idx, _text_idx)) in self
            .save_as_folder_shapes
            .iter()
            .enumerate()
            .take(max_visible_folders)
        {
            let actual_idx = display_idx + self.save_as_scroll_offset;
            let bg_color = if Some(actual_idx) == self.save_as_selected_folder {
                Color::new(150, 200, 255)
            } else {
                Color::WHITE
            };

            // Update background color
            surface.update_rectangle_color(*bg_idx, bg_color);
        }
    }

    pub fn trigger_save_as(&mut self) {
        self.enter_save_as_mode();
    }
}

impl Application for Notepad {
    fn init(&mut self, surface: &mut Surface) {
        // Text content display
        self.text_area_idx = surface.add_shape(Shape::Text {
            x: 5,
            y: 5,
            content: self.get_display_text(),
            color: Color::BLACK,
            background_color: Color::WHITE,
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Cursor (simple vertical line)
        self.cursor_idx = surface.add_shape(Shape::Rectangle {
            x: 5,
            y: 5,
            width: 1,
            height: 16,
            color: Color::BLACK,
            filled: true,
            hide: false,
        });

        self.update_display_lines();
    }

    fn handle_char_input(&mut self, ch: char, ctrl_pressed: bool, _surface: &mut Surface) {
        if self.mode == NotepadMode::SaveAs {
            // In Save-As mode, only handle filename input
            match ch {
                '\u{08}' => {
                    // Backspace
                    if !self.save_as_filename.is_empty() {
                        self.save_as_filename.pop();
                    }
                }
                '\r' | '\n' => {
                    if let Some(selected_folder) = self.save_as_selected_folder {
                        let folder_name = &self.save_as_folders[selected_folder].name;
                        self.handle_save_as_navigation(&folder_name.clone());
                    }
                }
                ch if ch.is_control() => {
                    // Ignore control characters
                }
                _ => {
                    // Add character to filename
                    if self.save_as_filename.len() < 50 {
                        // Limit filename length
                        self.save_as_filename.push(ch);
                    }
                }
            }
            return;
        }

        if ctrl_pressed {
            match ch {
                's' | 'S' => {
                    self.handle_save();
                }
                'o' | 'O' => {
                    unimplemented!("Ctrl+O - Open file dialog not implemented.");
                }
                _ => {}
            }
            return;
        }

        match ch {
            '\u{08}' => {
                // Backspace
                if self.cursor_position > 0 {
                    self.text_content.remove(self.cursor_position - 1);
                    self.cursor_position -= 1;
                }
            }
            '\r' | '\n' => {
                // Enter - add newline
                self.text_content.insert(self.cursor_position, '\n');
                self.cursor_position += 1;
            }
            ch if ch.is_control() => {
                // Ignore other control characters
            }
            _ => {
                // Regular character
                self.text_content.insert(self.cursor_position, ch);
                self.cursor_position += 1;
            }
        }

        self.has_changes = true;

        self.update_display_lines();
        self.update_scroll_if_needed();
    }

    fn handle_key_input(&mut self, key: KeyCode, _surface: &mut Surface) {
        if self.mode == NotepadMode::SaveAs {
            // In Save-As mode, handle folder navigation
            match key {
                KeyCode::ArrowUp => {
                    if let Some(selected) = self.save_as_selected_folder {
                        if selected > 0 {
                            self.save_as_selected_folder = Some(selected - 1);
                        }
                    } else if !self.save_as_folders.is_empty() {
                        self.save_as_selected_folder = Some(self.save_as_folders.len() - 1);
                    }
                }
                KeyCode::ArrowDown => {
                    if let Some(selected) = self.save_as_selected_folder {
                        if selected < self.save_as_folders.len() - 1 {
                            self.save_as_selected_folder = Some(selected + 1);
                        }
                    } else if !self.save_as_folders.is_empty() {
                        self.save_as_selected_folder = Some(0);
                    }
                }
                KeyCode::Escape => {
                    // Cancel Save-As dialog
                    self.mode = NotepadMode::Normal;
                }
                _ => {}
            }
            return;
        }

        match key {
            KeyCode::ArrowLeft => {
                if self.cursor_position > 0 {
                    self.cursor_position -= 1;
                }
            }
            KeyCode::ArrowRight => {
                if self.cursor_position < self.text_content.len() {
                    self.cursor_position += 1;
                }
            }
            _ => {}
        }
    }

    fn render(&mut self, surface: &mut Surface) {
        if self.mode == NotepadMode::SaveAs {
            self.render_save_as_dialog(surface);
            return;
        }

        if surface.shapes.len() > 2 {
            self.restore_normal_view(surface);
        }

        let current_display = self.get_display_text();

        // Only update if content changed
        if current_display != self.previous_content {
            surface.update_text_content(self.text_area_idx, current_display.clone(), None);
            self.previous_content = current_display;
        }

        // Update cursor position
        let (cursor_x, cursor_y) = self.get_cursor_visual_position();
        if cursor_x != self.prev_cursor_x || cursor_y != self.prev_cursor_y {
            surface.move_shape(self.cursor_idx, cursor_x, cursor_y);

            self.prev_cursor_x = cursor_x;
            self.prev_cursor_y = cursor_y;
        }
    }

    fn handle_mouse_click(&mut self, x: usize, y: usize, _surface: &mut Surface) {
        if self.mode != NotepadMode::SaveAs {
            return;
        }

        let width = 600; // Assume standard window width
        let height = 400; // Assume standard window height

        // Handle folder list clicks
        let list_start_y = 85;

        let max_visible_folders = 8;

        for (i, _folder) in self
            .save_as_folders
            .iter()
            .enumerate()
            .skip(self.save_as_scroll_offset)
            .take(max_visible_folders)
        {
            let y_pos = list_start_y + (i - self.save_as_scroll_offset) * 20;

            if x >= 30 && x < width - 50 && y >= y_pos && y < y_pos + 18 {
                if Some(i) == self.save_as_selected_folder {
                    // Double-click effect: navigate into folder
                    let folder_name = self.save_as_folders[i].name.clone();
                    self.handle_save_as_navigation(&folder_name);
                } else {
                    // Single click: select folder
                    serial_println!("Selected folder: {}", self.save_as_folders[i].name);
                    self.save_as_selected_folder = Some(i);
                }
                return;
            }
        }

        // Handle Save button click
        let button_y = height - 60;
        if x >= 30 && x < 100 && y >= button_y && y < button_y + 25 {
            self.perform_save_as();
        }

        // Handle Cancel button click
        if x >= 110 && x < 180 && y >= button_y && y < button_y + 25 {
            self.mode = NotepadMode::Normal;
        }
    }

    fn get_title(&self) -> Option<String> {
        let base_title = if let Some(ref path) = self.open_file_path {
            path.split('/').last().unwrap_or("Untitled").to_string()
        } else {
            "Untitled".to_string()
        };

        let title = if self.has_changes {
            format!("{}*", base_title)
        } else {
            base_title
        };

        Some(format!("{} - Notepad", title))
    }
}
