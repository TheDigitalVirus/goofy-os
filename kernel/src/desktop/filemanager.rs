use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use pc_keyboard::KeyCode;

use crate::{
    framebuffer::Color,
    fs::{
        fat32::FileEntry,
        manager::{
            copy_directory, copy_file, create_directory, create_file, delete_directory,
            delete_file, list_directory, move_item, rename_item,
        },
    },
    serial_println,
    surface::{Shape, Surface},
};
use noto_sans_mono_bitmap::{FontWeight, RasterHeight};

const FILE_LIST_HEIGHT: usize = 260;
const FILE_ENTRY_HEIGHT: usize = 20;
const BUTTON_HEIGHT: usize = 25;
const MARGIN: usize = 10;
const TEXT_INPUT_HEIGHT: usize = 25;

#[derive(Clone, Debug, PartialEq)]
pub enum FileManagerMode {
    Browse,
    NewFile,
    NewFolder,
    DeleteFile,
    DeleteFolder,
    ViewFile(FileEntry),
    Rename(FileEntry),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClipboardOperation {
    Copy,
    Cut,
}

#[derive(Clone, Debug)]
pub struct ClipboardEntry {
    pub file_path: String,
    pub file_name: String,
    pub is_directory: bool,
    pub operation: ClipboardOperation,
}

#[derive(Clone, Debug)]
pub struct DirectoryPath {
    pub path: String,
    pub name: String,
}

pub struct FileManager {
    mode: FileManagerMode,
    files: Vec<FileEntry>,
    selected_file_index: Option<usize>,
    scroll_offset: usize,
    input_text: String,
    status_message: String,
    open_file_options: Option<Vec<(usize, String)>>, // Y offset, name
    selected_open_file_app: Option<String>,

    // Clipboard for copy/cut/paste operations
    clipboard: Option<ClipboardEntry>,

    // Directory navigation - now path-based
    current_path: String,
    directory_stack: Vec<DirectoryPath>,

    // UI element indices
    status_text_idx: Option<usize>,
    input_text_idx: Option<usize>,
    breadcrumb_text_idx: Option<usize>,

    // Button indices
    new_file_btn_idx: Option<usize>,
    new_folder_btn_idx: Option<usize>,
    delete_file_btn_idx: Option<usize>,
    view_file_btn_idx: Option<usize>,
    back_btn_idx: Option<usize>,
    up_btn_idx: Option<usize>,
    create_btn_idx: Option<usize>,
    confirm_delete_btn_idx: Option<usize>,
    confirm_open_file_btn_idx: Option<usize>,

    // New clipboard operation buttons
    copy_btn_idx: Option<usize>,
    cut_btn_idx: Option<usize>,
    paste_btn_idx: Option<usize>,
    rename_btn_idx: Option<usize>,

    // File list UI tracking for optimized updates
    file_list_shapes: Vec<(usize, usize)>, // (background_idx, name_idx) for each file entry
    previous_selected_file: Option<usize>,
    ui_initialized: bool,
}

impl FileManager {
    pub fn new() -> Self {
        let mut fm = Self {
            mode: FileManagerMode::Browse,
            files: Vec::new(),
            selected_file_index: None,
            scroll_offset: 0,
            input_text: String::new(),
            status_message: "Ready".to_string(),
            open_file_options: None,
            selected_open_file_app: None,

            // Initialize clipboard as empty
            clipboard: None,

            // Initialize at root directory
            current_path: "/".to_string(),
            directory_stack: Vec::new(),

            status_text_idx: None,
            input_text_idx: None,
            breadcrumb_text_idx: None,

            new_file_btn_idx: None,
            new_folder_btn_idx: None,
            delete_file_btn_idx: None,
            view_file_btn_idx: None,
            back_btn_idx: None,
            up_btn_idx: None,
            create_btn_idx: None,
            confirm_delete_btn_idx: None,
            confirm_open_file_btn_idx: None,

            // Initialize new clipboard buttons
            copy_btn_idx: None,
            cut_btn_idx: None,
            paste_btn_idx: None,
            rename_btn_idx: None,

            file_list_shapes: Vec::new(),
            previous_selected_file: None,
            ui_initialized: false,
        };

        fm.refresh_file_list();
        fm
    }

    fn load_recomended_open_list(
        &self,
        file_name: &String,
    ) -> (Option<&'static str>, Vec<&'static str>) {
        let recomended = if file_name.to_lowercase().ends_with(".txt") {
            Some("notepad")
        } else {
            None
        };

        let other: Vec<&'static str> = if let Some(rec) = recomended {
            match rec {
                "notepad" => ["calculator"].to_vec(),
                _ => ["notepad", "calculator"].to_vec(),
            }
        } else {
            ["notepad", "calculator"].to_vec()
        };

        (recomended, other)
    }

    fn refresh_file_list(&mut self) {
        match list_directory(&self.current_path) {
            Ok(files) => {
                // Include both files and directories, filter out "." entry
                self.files = files.into_iter().filter(|f| f.name != ".").collect();
                self.status_message = format!("Found {} items", self.files.len());
                serial_println!("File Manager: Found {} items", self.files.len());
            }
            Err(e) => {
                self.status_message = format!("Error: {}", e);
                serial_println!("File Manager: Error listing directory: {}", e);
            }
        }
    }

    pub fn setup_ui(&mut self, surface: &mut Surface) {
        self.clear_ui(surface);

        match &self.mode {
            FileManagerMode::Browse => self.setup_browse_ui(surface),
            FileManagerMode::NewFile => self.setup_new_file_ui(surface),
            FileManagerMode::NewFolder => self.setup_new_folder_ui(surface),
            FileManagerMode::DeleteFile => self.setup_delete_file_ui(surface),
            FileManagerMode::DeleteFolder => self.setup_delete_folder_ui(surface),
            FileManagerMode::ViewFile(_) => self.setup_view_file_ui(surface),
            FileManagerMode::Rename(_) => self.setup_rename_ui(surface),
        }

        self.ui_initialized = true;
    }

    /// Update only the text input without rebuilding the entire UI
    pub fn update_input_text(&mut self, surface: &mut Surface) {
        if let Some(idx) = self.input_text_idx {
            surface.update_text_content(idx, format!("{}_", self.input_text), None);
        }
    }

    /// Update only the status message without rebuilding the entire UI
    pub fn update_status_message(&mut self, surface: &mut Surface, message: String) {
        self.status_message = message;
        if let Some(idx) = self.status_text_idx {
            surface.update_text_content(idx, self.status_message.clone(), None);
        }
    }

    /// Update file selection highlighting without rebuilding the entire UI
    pub fn update_file_selection(&mut self, surface: &mut Surface) {
        if !self.ui_initialized || self.mode != FileManagerMode::Browse {
            return;
        }

        // Update previous selection (if any) to normal color
        if let Some(prev_idx) = self.previous_selected_file {
            if prev_idx < self.file_list_shapes.len() {
                let (bg_idx, _) = self.file_list_shapes[prev_idx];
                surface.update_rectangle_color(bg_idx, Color::WHITE);
            }
        }

        // Update new selection (if any) to highlight color
        if let Some(curr_idx) = self.selected_file_index {
            if curr_idx < self.file_list_shapes.len() {
                let (bg_idx, _) = self.file_list_shapes[curr_idx];
                surface.update_rectangle_color(bg_idx, Color::new(150, 200, 255));
            }
        }

        self.previous_selected_file = self.selected_file_index;
    }
    fn clear_ui(&mut self, surface: &mut Surface) {
        surface.clear_all_shapes();

        self.status_text_idx = None;
        self.input_text_idx = None;
        self.breadcrumb_text_idx = None;
        self.open_file_options = None;

        self.new_file_btn_idx = None;
        self.new_folder_btn_idx = None;
        self.delete_file_btn_idx = None;
        self.view_file_btn_idx = None;
        self.back_btn_idx = None;
        self.up_btn_idx = None;
        self.create_btn_idx = None;
        self.confirm_delete_btn_idx = None;
        self.confirm_open_file_btn_idx = None;

        // Clear new clipboard button indices
        self.copy_btn_idx = None;
        self.cut_btn_idx = None;
        self.paste_btn_idx = None;
        self.rename_btn_idx = None;

        self.file_list_shapes.clear();
        self.previous_selected_file = None;
        self.ui_initialized = false;
    }

    fn setup_browse_ui(&mut self, surface: &mut Surface) {
        let width = surface.width;
        let height = surface.height;

        // Breadcrumb navigation
        let breadcrumb_text = self.get_breadcrumb_text();
        self.breadcrumb_text_idx = Some(surface.add_shape(Shape::Text {
            x: MARGIN,
            y: 10,
            content: breadcrumb_text,
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        }));

        // Up button (if not in root)
        if self.current_path != "/" {
            self.up_btn_idx = Some(surface.add_shape(Shape::Rectangle {
                x: width - 60,
                y: 8,
                width: 50,
                height: 20,
                color: Color::new(200, 200, 255),
                filled: true,
                hide: false,
            }));

            surface.add_shape(Shape::Rectangle {
                x: width - 60,
                y: 8,
                width: 50,
                height: 20,
                color: Color::BLACK,
                filled: false,
                hide: false,
            });

            surface.add_shape(Shape::Text {
                x: width - 55,
                y: 10,
                content: "Up".to_string(),
                color: Color::BLACK,
                background_color: Color::new(200, 200, 255),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });
        }

        // File list background
        // surface.add_shape(Shape::Rectangle {
        //     x: MARGIN,
        //     y: 40,
        //     width: width - 2 * MARGIN,
        //     height: FILE_LIST_HEIGHT,
        //     color: Color::WHITE,
        //     filled: true,
        //     hide: false,
        // });

        // // File list border
        // surface.add_shape(Shape::Rectangle {
        //     x: MARGIN,
        //     y: 40,
        //     width: width - 2 * MARGIN,
        //     height: FILE_LIST_HEIGHT,
        //     color: Color::BLACK,
        //     filled: false,
        //     hide: false,
        // });

        // Display files and folders
        let max_visible_files = FILE_LIST_HEIGHT / FILE_ENTRY_HEIGHT;

        for (i, file) in self
            .files
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(max_visible_files)
        {
            let y_pos = 45 + (i - self.scroll_offset) * FILE_ENTRY_HEIGHT;
            let bg_color = if Some(i) == self.selected_file_index {
                Color::new(150, 200, 255)
            } else {
                Color::WHITE
            };

            // File entry background
            let bg_idx = surface.add_shape(Shape::Rectangle {
                x: MARGIN + 2,
                y: y_pos,
                width: width - 2 * MARGIN - 4,
                height: FILE_ENTRY_HEIGHT - 2,
                color: bg_color,
                filled: true,
                hide: false,
            });

            // File/folder icon and name
            let display_name = if file.is_directory {
                format!("/{}", file.name)
            } else {
                file.name.clone()
            };

            let display_name = if display_name.len() > 35 {
                format!("{}...", &display_name[..32])
            } else {
                display_name
            };

            let name_idx = surface.add_shape(Shape::Text {
                x: MARGIN + 5,
                y: y_pos + 3,
                content: display_name,
                color: Color::BLACK,
                background_color: bg_color,
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            // File size (only for files)
            if !file.is_directory {
                let size_text = if file.size < 1024 {
                    format!("{} B", file.size)
                } else if file.size < 1024 * 1024 {
                    format!("{} KB", file.size / 1024)
                } else {
                    format!("{} MB", file.size / (1024 * 1024))
                };

                surface.add_shape(Shape::Text {
                    x: width - 80,
                    y: y_pos + 3,
                    content: size_text,
                    color: Color::BLACK,
                    background_color: bg_color,
                    font_size: RasterHeight::Size16,
                    font_weight: FontWeight::Regular,
                    hide: false,
                });
            }

            // Store shape indices for later updates
            self.file_list_shapes.push((bg_idx, name_idx));
        }

        // Buttons
        let button_y = height - 60;

        // New File button
        self.new_file_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: button_y,
            width: 65,
            height: BUTTON_HEIGHT,
            color: Color::new(220, 220, 220),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: button_y,
            width: 65,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 8,
            y: button_y + 5,
            content: "File".to_string(),
            color: Color::BLACK,
            background_color: Color::new(220, 220, 220),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // New Folder button
        self.new_folder_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN + 75,
            y: button_y,
            width: 65,
            height: BUTTON_HEIGHT,
            color: Color::new(180, 220, 255),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN + 75,
            y: button_y,
            width: 65,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 82,
            y: button_y + 5,
            content: "Folder".to_string(),
            color: Color::BLACK,
            background_color: Color::new(180, 220, 255),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Delete button
        self.delete_file_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN + 150,
            y: button_y,
            width: 65,
            height: BUTTON_HEIGHT,
            color: Color::new(255, 180, 180),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN + 150,
            y: button_y,
            width: 65,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 165,
            y: button_y + 5,
            content: "Delete".to_string(),
            color: Color::BLACK,
            background_color: Color::new(255, 180, 180),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Open button
        self.view_file_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN + 225,
            y: button_y,
            width: 65,
            height: BUTTON_HEIGHT,
            color: Color::new(180, 255, 180),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN + 225,
            y: button_y,
            width: 65,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 245,
            y: button_y + 5,
            content: "Open".to_string(),
            color: Color::BLACK,
            background_color: Color::new(180, 255, 180),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Second row of buttons for clipboard operations
        let button_y2 = height - 90;

        // Copy button
        self.copy_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: button_y2,
            width: 50,
            height: BUTTON_HEIGHT,
            color: Color::new(200, 255, 200),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: button_y2,
            width: 50,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 15,
            y: button_y2 + 5,
            content: "Copy".to_string(),
            color: Color::BLACK,
            background_color: Color::new(200, 255, 200),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Cut button
        self.cut_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN + 60,
            y: button_y2,
            width: 50,
            height: BUTTON_HEIGHT,
            color: Color::new(255, 220, 150),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN + 60,
            y: button_y2,
            width: 50,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 78,
            y: button_y2 + 5,
            content: "Cut".to_string(),
            color: Color::BLACK,
            background_color: Color::new(255, 220, 150),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Paste button (only enabled if clipboard has content)
        let paste_color = if self.clipboard.is_some() {
            Color::new(255, 200, 255)
        } else {
            Color::new(200, 200, 200)
        };

        self.paste_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN + 120,
            y: button_y2,
            width: 50,
            height: BUTTON_HEIGHT,
            color: paste_color,
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN + 120,
            y: button_y2,
            width: 50,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 135,
            y: button_y2 + 5,
            content: "Paste".to_string(),
            color: Color::BLACK,
            background_color: paste_color,
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Rename button
        self.rename_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN + 180,
            y: button_y2,
            width: 60,
            height: BUTTON_HEIGHT,
            color: Color::new(200, 200, 255),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN + 180,
            y: button_y2,
            width: 60,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 195,
            y: button_y2 + 5,
            content: "Rename".to_string(),
            color: Color::BLACK,
            background_color: Color::new(200, 200, 255),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Status bar
        self.status_text_idx = Some(surface.add_shape(Shape::Text {
            x: MARGIN,
            y: height - 25,
            content: self.status_message.clone(),
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        }));
    }

    fn setup_new_folder_ui(&mut self, surface: &mut Surface) {
        let width = surface.width;
        let height = surface.height;

        // Title
        surface.add_shape(Shape::Text {
            x: MARGIN,
            y: 50,
            content: "Create New Folder".to_string(),
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Bold,
            hide: false,
        });

        // Folder name input label
        surface.add_shape(Shape::Text {
            x: MARGIN,
            y: 80,
            content: "Folder name:".to_string(),
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Folder name input background
        surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: 100,
            width: width - 2 * MARGIN,
            height: TEXT_INPUT_HEIGHT,
            color: Color::WHITE,
            filled: true,
            hide: false,
        });

        surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: 100,
            width: width - 2 * MARGIN,
            height: TEXT_INPUT_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        // Folder name input text
        self.input_text_idx = Some(surface.add_shape(Shape::Text {
            x: MARGIN + 5,
            y: 105,
            content: format!("{}_", self.input_text),
            color: Color::BLACK,
            background_color: Color::WHITE,
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        }));

        // Buttons
        let button_y = height - 60;

        // Create button
        self.create_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: button_y,
            width: 80,
            height: BUTTON_HEIGHT,
            color: Color::new(180, 255, 180),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: button_y,
            width: 80,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 20,
            y: button_y + 5,
            content: "Create".to_string(),
            color: Color::BLACK,
            background_color: Color::new(180, 255, 180),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Back button
        self.back_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN + 90,
            y: button_y,
            width: 80,
            height: BUTTON_HEIGHT,
            color: Color::new(220, 220, 220),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN + 90,
            y: button_y,
            width: 80,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 120,
            y: button_y + 5,
            content: "Cancel".to_string(),
            color: Color::BLACK,
            background_color: Color::new(220, 220, 220),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Status bar
        self.status_text_idx = Some(surface.add_shape(Shape::Text {
            x: MARGIN,
            y: height - 25,
            content: self.status_message.clone(),
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        }));
    }

    fn setup_delete_folder_ui(&mut self, surface: &mut Surface) {
        let height = surface.height;

        let selected_name = if let Some(index) = self.selected_file_index {
            if index < self.files.len() {
                self.files[index].name.clone()
            } else {
                "Unknown".to_string()
            }
        } else {
            "None".to_string()
        };

        // Title
        surface.add_shape(Shape::Text {
            x: MARGIN,
            y: 50,
            content: "Delete Folder".to_string(),
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Bold,
            hide: false,
        });

        // Confirmation message
        surface.add_shape(Shape::Text {
            x: MARGIN,
            y: 80,
            content: format!(
                "Are you sure you want to delete folder '{}'?",
                selected_name
            ),
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN,
            y: 100,
            content: "Warning: This will only work if the folder is empty!".to_string(),
            color: Color::new(200, 0, 0),
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Buttons
        let button_y = height - 60;

        // Confirm Delete button
        self.confirm_delete_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: button_y,
            width: 100,
            height: BUTTON_HEIGHT,
            color: Color::new(255, 180, 180),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: button_y,
            width: 100,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 10,
            y: button_y + 5,
            content: "Delete Folder".to_string(),
            color: Color::BLACK,
            background_color: Color::new(255, 180, 180),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Cancel button
        self.back_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN + 110,
            y: button_y,
            width: 80,
            height: BUTTON_HEIGHT,
            color: Color::new(220, 220, 220),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN + 110,
            y: button_y,
            width: 80,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 135,
            y: button_y + 5,
            content: "Cancel".to_string(),
            color: Color::BLACK,
            background_color: Color::new(220, 220, 220),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Status bar
        self.status_text_idx = Some(surface.add_shape(Shape::Text {
            x: MARGIN,
            y: height - 25,
            content: self.status_message.clone(),
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        }));
    }

    // Navigation methods
    pub fn navigate_up(&mut self) -> Result<(), &'static str> {
        if let Some(parent_dir) = self.directory_stack.pop() {
            self.current_path = parent_dir.path;
            self.refresh_file_list();
            self.selected_file_index = None;
            Ok(())
        } else if self.current_path != "/" {
            // Navigate to root if we're not already there
            self.current_path = "/".to_string();
            self.refresh_file_list();
            self.selected_file_index = None;
            Ok(())
        } else {
            Err("Already at root directory")
        }
    }

    pub fn navigate_into_directory(&mut self, dir_name: &str) -> Result<(), &'static str> {
        serial_println!("Navigating into directory: {}", dir_name);

        if dir_name == ".." {
            return self.navigate_up();
        }

        // Find the directory in current directory
        let dir_entry = self
            .files
            .iter()
            .find(|f| f.is_directory && f.name == dir_name);

        if let Some(_dir) = dir_entry {
            // Save current directory to stack
            self.directory_stack.push(DirectoryPath {
                path: self.current_path.clone(),
                name: if self.current_path == "/" {
                    "Root".to_string()
                } else {
                    self.current_path
                        .split('/')
                        .last()
                        .unwrap_or("Unknown")
                        .to_string()
                },
            });

            // Navigate to the directory
            self.current_path = if self.current_path == "/" {
                format!("/{}", dir_name)
            } else {
                format!("{}/{}", self.current_path, dir_name)
            };

            self.refresh_file_list();
            self.selected_file_index = None;
            Ok(())
        } else {
            Err("Directory not found or not a directory")
        }
    }

    fn setup_new_file_ui(&mut self, surface: &mut Surface) {
        let width = surface.width;
        let height = surface.height;

        // Title
        surface.add_shape(Shape::Text {
            x: MARGIN,
            y: 50,
            content: "Create New File".to_string(),
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Bold,
            hide: false,
        });

        // Filename input label
        surface.add_shape(Shape::Text {
            x: MARGIN,
            y: 80,
            content: "Filename:".to_string(),
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Filename input background
        surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: 100,
            width: width - 2 * MARGIN,
            height: TEXT_INPUT_HEIGHT,
            color: Color::WHITE,
            filled: true,
            hide: false,
        });

        surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: 100,
            width: width - 2 * MARGIN,
            height: TEXT_INPUT_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        // Filename input text
        self.input_text_idx = Some(surface.add_shape(Shape::Text {
            x: MARGIN + 5,
            y: 105,
            content: format!("{}_", self.input_text),
            color: Color::BLACK,
            background_color: Color::WHITE,
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        }));

        // Buttons
        let button_y = height - 60;

        // Create button
        self.create_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: button_y,
            width: 80,
            height: BUTTON_HEIGHT,
            color: Color::new(180, 255, 180),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN,
            y: button_y,
            width: 80,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 20,
            y: button_y + 5,
            content: "Create".to_string(),
            color: Color::BLACK,
            background_color: Color::new(180, 255, 180),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Back button
        self.back_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: MARGIN + 90,
            y: button_y,
            width: 80,
            height: BUTTON_HEIGHT,
            color: Color::new(220, 220, 220),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: MARGIN + 90,
            y: button_y,
            width: 80,
            height: BUTTON_HEIGHT,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: MARGIN + 115,
            y: button_y + 5,
            content: "Back".to_string(),
            color: Color::BLACK,
            background_color: Color::new(220, 220, 220),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Status
        self.status_text_idx = Some(surface.add_shape(Shape::Text {
            x: MARGIN,
            y: height - 25,
            content: "Enter filename and content, then click Create".to_string(),
            color: Color::BLACK,
            background_color: Color::new(240, 240, 240),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        }));
    }

    fn setup_delete_file_ui(&mut self, surface: &mut Surface) {
        let height = surface.height;

        if let Some(idx) = self.selected_file_index {
            if let Some(file) = self.files.get(idx) {
                // Title
                surface.add_shape(Shape::Text {
                    x: MARGIN,
                    y: 50,
                    content: "Delete File".to_string(),
                    color: Color::BLACK,
                    background_color: Color::new(240, 240, 240),
                    font_size: RasterHeight::Size16,
                    font_weight: FontWeight::Bold,
                    hide: false,
                });

                // Confirmation message
                surface.add_shape(Shape::Text {
                    x: MARGIN,
                    y: 100,
                    content: format!("Are you sure you want to delete '{}'?", file.name),
                    color: Color::BLACK,
                    background_color: Color::new(240, 240, 240),
                    font_size: RasterHeight::Size16,
                    font_weight: FontWeight::Regular,
                    hide: false,
                });

                surface.add_shape(Shape::Text {
                    x: MARGIN,
                    y: 130,
                    content: "This action cannot be undone!".to_string(),
                    color: Color::new(200, 0, 0),
                    background_color: Color::new(240, 240, 240),
                    font_size: RasterHeight::Size16,
                    font_weight: FontWeight::Bold,
                    hide: false,
                });

                // Buttons
                let button_y = height - 60;

                // Confirm Delete button
                self.confirm_delete_btn_idx = Some(surface.add_shape(Shape::Rectangle {
                    x: MARGIN,
                    y: button_y,
                    width: 100,
                    height: BUTTON_HEIGHT,
                    color: Color::new(255, 100, 100),
                    filled: true,
                    hide: false,
                }));

                surface.add_shape(Shape::Rectangle {
                    x: MARGIN,
                    y: button_y,
                    width: 100,
                    height: BUTTON_HEIGHT,
                    color: Color::BLACK,
                    filled: false,
                    hide: false,
                });

                surface.add_shape(Shape::Text {
                    x: MARGIN + 15,
                    y: button_y + 5,
                    content: "Yes, Delete".to_string(),
                    color: Color::BLACK,
                    background_color: Color::new(255, 100, 100),
                    font_size: RasterHeight::Size16,
                    font_weight: FontWeight::Regular,
                    hide: false,
                });

                // Back button
                self.back_btn_idx = Some(surface.add_shape(Shape::Rectangle {
                    x: MARGIN + 110,
                    y: button_y,
                    width: 80,
                    height: BUTTON_HEIGHT,
                    color: Color::new(220, 220, 220),
                    filled: true,
                    hide: false,
                }));

                surface.add_shape(Shape::Rectangle {
                    x: MARGIN + 110,
                    y: button_y,
                    width: 80,
                    height: BUTTON_HEIGHT,
                    color: Color::BLACK,
                    filled: false,
                    hide: false,
                });

                surface.add_shape(Shape::Text {
                    x: MARGIN + 135,
                    y: button_y + 5,
                    content: "Cancel".to_string(),
                    color: Color::BLACK,
                    background_color: Color::new(220, 220, 220),
                    font_size: RasterHeight::Size16,
                    font_weight: FontWeight::Regular,
                    hide: false,
                });
            }
        }
    }

    fn setup_view_file_ui(&mut self, surface: &mut Surface) {
        let height = surface.height;

        if let FileManagerMode::ViewFile(file) = &self.mode {
            // Title
            surface.add_shape(Shape::Text {
                x: MARGIN,
                y: 50,
                content: format!("Select an application to open: {}", file.name),
                color: Color::BLACK,
                background_color: Color::new(240, 240, 240),
                font_size: RasterHeight::Size20,
                font_weight: FontWeight::Bold,
                hide: false,
            });

            surface.add_shape(Shape::Text {
                x: MARGIN,
                y: 70,
                content: "Recomended:".to_string(),
                color: Color::BLACK,
                background_color: Color::new(240, 240, 240),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Bold,
                hide: false,
            });

            let (recommended, all) = self.load_recomended_open_list(&file.name);

            if recommended.is_some() && self.selected_open_file_app.is_none() {
                self.selected_open_file_app = recommended.map(|s| s.to_string());
            }

            if recommended.is_some()
                && self.selected_open_file_app == recommended.map(|s| s.to_string())
            {
                surface.add_shape(Shape::Rectangle {
                    x: MARGIN,
                    y: 90,
                    width: 200,
                    height: 20,
                    color: Color::new(150, 200, 255),
                    filled: true,
                    hide: false,
                });
            }

            self.open_file_options = Some(Vec::new());
            if recommended.is_some() {
                self.open_file_options
                    .as_mut()
                    .unwrap()
                    .push((90, recommended.unwrap().to_string()));
            }

            surface.add_shape(Shape::Text {
                x: MARGIN,
                y: 90,
                content: recommended
                    .unwrap_or("No recommended apps found")
                    .to_string(),
                color: Color::BLACK,
                background_color: Color::new(240, 240, 240),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            surface.add_shape(Shape::Text {
                x: MARGIN,
                y: 110,
                content: "Other:".to_string(),
                color: Color::BLACK,
                background_color: Color::new(240, 240, 240),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Bold,
                hide: false,
            });

            for (i, app) in all.iter().enumerate() {
                if self.selected_open_file_app == Some(app.to_string()) {
                    surface.add_shape(Shape::Rectangle {
                        x: MARGIN,
                        y: 130 + i * 20,
                        width: 200,
                        height: 20,
                        color: Color::new(150, 200, 255),
                        filled: true,
                        hide: false,
                    });
                }

                surface.add_shape(Shape::Text {
                    x: MARGIN,
                    y: 130 + i * 20,
                    content: app.to_string(),
                    color: Color::BLACK,
                    background_color: Color::new(240, 240, 240),
                    font_size: RasterHeight::Size16,
                    font_weight: FontWeight::Regular,
                    hide: false,
                });

                self.open_file_options
                    .as_mut()
                    .unwrap()
                    .push((130 + i * 20, app.to_string()));
            }

            // Back button
            let button_y = height - 60;
            self.back_btn_idx = Some(surface.add_shape(Shape::Rectangle {
                x: MARGIN,
                y: button_y,
                width: 80,
                height: BUTTON_HEIGHT,
                color: Color::new(220, 220, 220),
                filled: true,
                hide: false,
            }));

            surface.add_shape(Shape::Rectangle {
                x: MARGIN,
                y: button_y,
                width: 80,
                height: BUTTON_HEIGHT,
                color: Color::BLACK,
                filled: false,
                hide: false,
            });

            surface.add_shape(Shape::Text {
                x: MARGIN + 25,
                y: button_y + 5,
                content: "Back".to_string(),
                color: Color::BLACK,
                background_color: Color::new(220, 220, 220),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            self.confirm_open_file_btn_idx = Some(surface.add_shape(Shape::Rectangle {
                x: MARGIN + 90,
                y: button_y,
                width: 80,
                height: BUTTON_HEIGHT,
                color: Color::new(180, 255, 180),
                filled: true,
                hide: false,
            }));

            surface.add_shape(Shape::Rectangle {
                x: MARGIN + 90,
                y: button_y,
                width: 80,
                height: BUTTON_HEIGHT,
                color: Color::BLACK,
                filled: false,
                hide: false,
            });

            surface.add_shape(Shape::Text {
                x: MARGIN + 115,
                y: button_y + 5,
                content: "Open".to_string(),
                color: Color::BLACK,
                background_color: Color::new(180, 255, 180),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });
        }
    }

    fn setup_rename_ui(&mut self, surface: &mut Surface) {
        let width = surface.width;
        let height = surface.height;

        if let FileManagerMode::Rename(file) = &self.mode {
            // Title
            surface.add_shape(Shape::Text {
                x: MARGIN,
                y: 50,
                content: "Rename Item".to_string(),
                color: Color::BLACK,
                background_color: Color::new(240, 240, 240),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Bold,
                hide: false,
            });

            // Current name display
            surface.add_shape(Shape::Text {
                x: MARGIN,
                y: 80,
                content: format!("Current name: {}", file.name),
                color: Color::BLACK,
                background_color: Color::new(240, 240, 240),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            // New name input label
            surface.add_shape(Shape::Text {
                x: MARGIN,
                y: 110,
                content: "New name:".to_string(),
                color: Color::BLACK,
                background_color: Color::new(240, 240, 240),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            // New name input background
            surface.add_shape(Shape::Rectangle {
                x: MARGIN,
                y: 130,
                width: width - 2 * MARGIN,
                height: TEXT_INPUT_HEIGHT,
                color: Color::WHITE,
                filled: true,
                hide: false,
            });

            surface.add_shape(Shape::Rectangle {
                x: MARGIN,
                y: 130,
                width: width - 2 * MARGIN,
                height: TEXT_INPUT_HEIGHT,
                color: Color::BLACK,
                filled: false,
                hide: false,
            });

            // New name input text
            self.input_text_idx = Some(surface.add_shape(Shape::Text {
                x: MARGIN + 5,
                y: 135,
                content: format!("{}_", self.input_text),
                color: Color::BLACK,
                background_color: Color::WHITE,
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            }));

            // Buttons
            let button_y = height - 60;

            // Rename button
            self.create_btn_idx = Some(surface.add_shape(Shape::Rectangle {
                x: MARGIN,
                y: button_y,
                width: 80,
                height: BUTTON_HEIGHT,
                color: Color::new(200, 200, 255),
                filled: true,
                hide: false,
            }));

            surface.add_shape(Shape::Rectangle {
                x: MARGIN,
                y: button_y,
                width: 80,
                height: BUTTON_HEIGHT,
                color: Color::BLACK,
                filled: false,
                hide: false,
            });

            surface.add_shape(Shape::Text {
                x: MARGIN + 20,
                y: button_y + 5,
                content: "Rename".to_string(),
                color: Color::BLACK,
                background_color: Color::new(200, 200, 255),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            // Cancel button
            self.back_btn_idx = Some(surface.add_shape(Shape::Rectangle {
                x: MARGIN + 90,
                y: button_y,
                width: 80,
                height: BUTTON_HEIGHT,
                color: Color::new(220, 220, 220),
                filled: true,
                hide: false,
            }));

            surface.add_shape(Shape::Rectangle {
                x: MARGIN + 90,
                y: button_y,
                width: 80,
                height: BUTTON_HEIGHT,
                color: Color::BLACK,
                filled: false,
                hide: false,
            });

            surface.add_shape(Shape::Text {
                x: MARGIN + 115,
                y: button_y + 5,
                content: "Cancel".to_string(),
                color: Color::BLACK,
                background_color: Color::new(220, 220, 220),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            });

            // Status bar
            self.status_text_idx = Some(surface.add_shape(Shape::Text {
                x: MARGIN,
                y: height - 25,
                content: "Enter new name and click Rename".to_string(),
                color: Color::BLACK,
                background_color: Color::new(240, 240, 240),
                font_size: RasterHeight::Size16,
                font_weight: FontWeight::Regular,
                hide: false,
            }));
        }
    }

    pub fn handle_click(
        &mut self,
        x: usize,
        y: usize,
        surface: &mut Surface,
    ) -> (bool, Option<(String, String)>) {
        match &self.mode {
            FileManagerMode::Browse => (self.handle_browse_click(x, y, surface), None),
            FileManagerMode::NewFile => (self.handle_new_file_click(x, y, surface), None),
            FileManagerMode::NewFolder => (self.handle_new_folder_click(x, y, surface), None),
            FileManagerMode::DeleteFile => (self.handle_delete_click(x, y, surface), None),
            FileManagerMode::DeleteFolder => (self.handle_delete_folder_click(x, y, surface), None),
            FileManagerMode::ViewFile(_) => self.handle_view_click(x, y, surface),
            FileManagerMode::Rename(_) => (self.handle_rename_click(x, y, surface), None),
        }
    }

    fn handle_browse_click(&mut self, x: usize, y: usize, surface: &mut Surface) -> bool {
        // Check file list clicks
        if x >= MARGIN && x < surface.width - MARGIN && y >= 45 && y < 45 + FILE_LIST_HEIGHT {
            let clicked_index = self.scroll_offset + (y - 45) / FILE_ENTRY_HEIGHT;
            if clicked_index < self.files.len() {
                self.selected_file_index = Some(clicked_index);
                // Use optimized update instead of full UI rebuild
                self.update_file_selection(surface);
                return true;
            }
        }

        // Check up button click
        if self.up_btn_idx.is_some() {
            if self.is_button_clicked(x, y, surface.width - 60, 8, 50, 20) {
                if let Ok(_) = self.navigate_up() {
                    self.setup_ui(surface);
                } else {
                    self.update_status_message(surface, "Already at root directory".to_string());
                }
                return true;
            }
        }

        // Check button clicks
        if self.new_file_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN, surface.height - 60, 65, BUTTON_HEIGHT) {
                self.mode = FileManagerMode::NewFile;
                self.input_text.clear();
                self.setup_ui(surface);
                return true;
            }
        }

        if self.new_folder_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 75, surface.height - 60, 65, BUTTON_HEIGHT) {
                self.mode = FileManagerMode::NewFolder;
                self.input_text.clear();
                self.setup_ui(surface);
                return true;
            }
        }

        if self.delete_file_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 150, surface.height - 60, 65, BUTTON_HEIGHT) {
                if let Some(idx) = self.selected_file_index {
                    if let Some(file) = self.files.get(idx) {
                        if file.is_directory {
                            self.mode = FileManagerMode::DeleteFolder;
                        } else {
                            self.mode = FileManagerMode::DeleteFile;
                        }
                        self.setup_ui(surface);
                    }
                } else {
                    // Use optimized status update instead of full UI rebuild
                    self.update_status_message(
                        surface,
                        "Please select a file or folder to delete".to_string(),
                    );
                }
                return true;
            }
        }

        if self.view_file_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 225, surface.height - 60, 65, BUTTON_HEIGHT) {
                return self.handle_view_file(surface);
            }
        }

        // Check clipboard operation buttons (second row)
        let button_y2 = surface.height - 90;

        // Copy button
        if self.copy_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN, button_y2, 50, BUTTON_HEIGHT) {
                return self.handle_copy_click(surface);
            }
        }

        // Cut button
        if self.cut_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 60, button_y2, 50, BUTTON_HEIGHT) {
                return self.handle_cut_click(surface);
            }
        }

        // Paste button
        if self.paste_btn_idx.is_some() && self.clipboard.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 120, button_y2, 50, BUTTON_HEIGHT) {
                return self.handle_paste_click(surface);
            }
        }

        // Rename button
        if self.rename_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 180, button_y2, 60, BUTTON_HEIGHT) {
                return self.handle_rename_click_from_browse(surface);
            }
        }

        false
    }

    fn handle_view_file(&mut self, surface: &mut Surface) -> bool {
        if let Some(idx) = self.selected_file_index {
            if let Some(file) = self.files.get(idx).cloned() {
                if file.is_directory {
                    // Navigate into directory
                    if let Ok(_) = self.navigate_into_directory(&file.name) {
                        self.setup_ui(surface);
                    } else {
                        self.update_status_message(
                            surface,
                            "Failed to enter directory".to_string(),
                        );
                    }
                } else {
                    // Open file
                    self.mode = FileManagerMode::ViewFile(file);
                    self.setup_ui(surface);
                }
            }
        } else {
            // Use optimized status update instead of full UI rebuild
            self.update_status_message(
                surface,
                "Please select a file or folder to open".to_string(),
            );
        }
        return true;
    }

    fn handle_new_file_click(&mut self, x: usize, y: usize, surface: &mut Surface) -> bool {
        if self.create_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN, surface.height - 60, 80, BUTTON_HEIGHT) {
                self.create_file(surface);
                return true;
            }
        }

        if self.back_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 90, surface.height - 60, 80, BUTTON_HEIGHT) {
                self.mode = FileManagerMode::Browse;
                self.setup_ui(surface);
                return true;
            }
        }

        false
    }

    fn handle_new_folder_click(&mut self, x: usize, y: usize, surface: &mut Surface) -> bool {
        if self.create_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN, surface.height - 60, 80, BUTTON_HEIGHT) {
                self.create_folder(surface);
                return true;
            }
        }

        if self.back_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 90, surface.height - 60, 80, BUTTON_HEIGHT) {
                self.mode = FileManagerMode::Browse;
                self.setup_ui(surface);
                return true;
            }
        }

        false
    }

    fn handle_delete_folder_click(&mut self, x: usize, y: usize, surface: &mut Surface) -> bool {
        if self.confirm_delete_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN, surface.height - 60, 100, BUTTON_HEIGHT) {
                self.delete_selected_folder(surface);
                return true;
            }
        }

        if self.back_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 110, surface.height - 60, 80, BUTTON_HEIGHT) {
                self.mode = FileManagerMode::Browse;
                self.setup_ui(surface);
                return true;
            }
        }

        false
    }

    fn create_folder(&mut self, surface: &mut Surface) {
        if self.input_text.trim().is_empty() {
            self.update_status_message(surface, "Please enter a folder name".to_string());
            return;
        }

        let folder_path = if self.current_path == "/" {
            format!("/{}", self.input_text)
        } else {
            format!("{}/{}", self.current_path, self.input_text)
        };

        match create_directory(&folder_path) {
            Ok(_) => {
                self.refresh_file_list();
                self.mode = FileManagerMode::Browse;
                self.input_text.clear();
                self.setup_ui(surface);
                self.update_status_message(surface, "Folder created successfully".to_string());
            }
            Err(e) => {
                self.update_status_message(surface, format!("Error creating folder: {}", e));
            }
        }
    }

    fn delete_selected_folder(&mut self, surface: &mut Surface) {
        if let Some(index) = self.selected_file_index {
            if let Some(folder) = self.files.get(index) {
                if !folder.is_directory {
                    self.update_status_message(
                        surface,
                        "Selected item is not a folder".to_string(),
                    );
                    return;
                }

                let folder_path = if self.current_path == "/" {
                    format!("/{}", folder.name)
                } else {
                    format!("{}/{}", self.current_path, folder.name)
                };

                match delete_directory(&folder_path) {
                    Ok(_) => {
                        self.refresh_file_list();
                        self.selected_file_index = None;
                        self.mode = FileManagerMode::Browse;
                        self.setup_ui(surface);
                        self.update_status_message(
                            surface,
                            "Folder deleted successfully".to_string(),
                        );
                    }
                    Err(e) => {
                        self.update_status_message(
                            surface,
                            format!("Error deleting folder: {}", e),
                        );
                    }
                }
            }
        }
    }

    fn handle_delete_click(&mut self, x: usize, y: usize, surface: &mut Surface) -> bool {
        if self.confirm_delete_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN, surface.height - 60, 100, BUTTON_HEIGHT) {
                self.delete_selected_file(surface);
                return true;
            }
        }

        if self.back_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 110, surface.height - 60, 80, BUTTON_HEIGHT) {
                self.mode = FileManagerMode::Browse;
                self.setup_ui(surface);
                return true;
            }
        }

        false
    }

    fn handle_view_click(
        &mut self,
        x: usize,
        y: usize,
        surface: &mut Surface,
    ) -> (bool, Option<(String, String)>) {
        if self.back_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN, surface.height - 60, 80, BUTTON_HEIGHT) {
                self.mode = FileManagerMode::Browse;
                self.setup_ui(surface);

                return (true, None);
            }
        }

        if self.confirm_open_file_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 90, surface.height - 60, 80, BUTTON_HEIGHT) {
                if let Some(app) = self.selected_open_file_app.clone() {
                    let file = self
                        .files
                        .get(self.selected_file_index.unwrap())
                        .cloned()
                        .unwrap();

                    let file_path = if self.current_path == "/" {
                        format!("/{}", file.name)
                    } else {
                        format!("{}/{}", self.current_path, file.name)
                    };

                    self.selected_open_file_app = None;
                    self.mode = FileManagerMode::Browse;
                    self.setup_ui(surface);

                    return (true, Some((file_path, app)));
                } else {
                    // Use optimized status update instead of full UI rebuild
                    self.update_status_message(
                        surface,
                        "Please select an application to open the file".to_string(),
                    );
                }
                return (true, None);
            }
        }

        if let Some(apps) = &self.open_file_options.clone() {
            for (app_y, app) in apps {
                if self.is_button_clicked(x, y, MARGIN, *app_y, 200, 20) {
                    self.selected_open_file_app = Some(app.to_string());
                    self.setup_ui(surface);
                    return (true, None);
                }
            }
        }

        (false, None)
    }

    fn is_button_clicked(
        &self,
        x: usize,
        y: usize,
        btn_x: usize,
        btn_y: usize,
        btn_width: usize,
        btn_height: usize,
    ) -> bool {
        x >= btn_x && x < btn_x + btn_width && y >= btn_y && y < btn_y + btn_height
    }

    fn create_file(&mut self, surface: &mut Surface) {
        if self.input_text.is_empty() {
            // Use optimized status update instead of full UI rebuild
            self.update_status_message(surface, "Please enter a filename".to_string());
            return;
        }

        let file_path = if self.current_path == "/" {
            format!("/{}", self.input_text)
        } else {
            format!("{}/{}", self.current_path, self.input_text)
        };

        match create_file(&file_path, &[]) {
            Ok(_) => {
                self.status_message = format!("File '{}' created successfully", self.input_text);
                self.refresh_file_list();
                self.mode = FileManagerMode::Browse;
                self.input_text.clear();
                self.setup_ui(surface);
            }
            Err(e) => {
                // Use optimized status update instead of full UI rebuild
                self.update_status_message(surface, format!("Error creating file: {}", e));
            }
        }
    }

    fn delete_selected_file(&mut self, surface: &mut Surface) {
        if let Some(idx) = self.selected_file_index {
            if let Some(file) = self.files.get(idx) {
                if file.is_directory {
                    self.update_status_message(
                        surface,
                        "Use Delete Folder for directories".to_string(),
                    );
                    return;
                }

                let filename = file.name.clone();
                let file_path = if self.current_path == "/" {
                    format!("/{}", filename)
                } else {
                    format!("{}/{}", self.current_path, filename)
                };

                match delete_file(&file_path) {
                    Ok(_) => {
                        self.status_message = format!("File '{}' deleted successfully", filename);
                        self.refresh_file_list();
                        self.selected_file_index = None;
                        self.mode = FileManagerMode::Browse;
                        self.setup_ui(surface);
                    }
                    Err(e) => {
                        self.status_message = format!("Error deleting file: {}", e);
                        self.mode = FileManagerMode::Browse;
                        self.setup_ui(surface);
                    }
                }
            }
        }
    }

    pub fn handle_char_input(&mut self, c: char, surface: &mut Surface) {
        match &self.mode {
            FileManagerMode::Browse => {
                if c == '\n' {
                    self.handle_view_file(surface);
                }
            }
            FileManagerMode::NewFile => {
                if c == '\x08' {
                    // Backspace
                    self.input_text.pop();
                } else if c == '\n' {
                    // Enter key, create file
                    self.create_file(surface);
                    return; // create_file handles its own UI updates
                } else if c.is_ascii() && !c.is_control() {
                    self.input_text.push(c);
                }

                // Use optimized text update instead of full UI rebuild
                self.update_input_text(surface);
            }
            FileManagerMode::NewFolder => {
                if c == '\x08' {
                    // Backspace
                    self.input_text.pop();
                } else if c == '\n' {
                    // Enter key, create folder
                    self.create_folder(surface);
                    return; // create_folder handles its own UI updates
                } else if c.is_ascii() && !c.is_control() {
                    self.input_text.push(c);
                }

                // Use optimized text update instead of full UI rebuild
                self.update_input_text(surface);
            }
            FileManagerMode::Rename(_) => {
                if c == '\x08' {
                    // Backspace
                    self.input_text.pop();
                } else if c == '\n' {
                    // Enter key, perform rename
                    self.perform_rename(surface);
                    return; // perform_rename handles its own UI updates
                } else if c.is_ascii() && !c.is_control() {
                    self.input_text.push(c);
                }

                // Use optimized text update instead of full UI rebuild
                self.update_input_text(surface);
            }
            _ => {}
        }
    }

    pub fn handle_key_input(&mut self, key: KeyCode, surface: &mut Surface) {
        match &self.mode {
            FileManagerMode::NewFile | FileManagerMode::NewFolder | FileManagerMode::Rename(_) => {
                match key {
                    KeyCode::Backspace => {
                        self.input_text.pop();
                        // Use optimized text update instead of full UI rebuild
                        self.update_input_text(surface);
                    }
                    KeyCode::Return => match &self.mode {
                        FileManagerMode::NewFile => self.create_file(surface),
                        FileManagerMode::NewFolder => self.create_folder(surface),
                        FileManagerMode::Rename(_) => self.perform_rename(surface),
                        _ => {}
                    },
                    _ => {}
                }
            }
            FileManagerMode::Browse => match key {
                KeyCode::ArrowUp => {
                    if let Some(ref mut idx) = self.selected_file_index {
                        if *idx > 0 {
                            *idx -= 1;
                            // Use optimized selection update instead of full UI rebuild
                            self.update_file_selection(surface);
                        }
                    } else if !self.files.is_empty() {
                        self.selected_file_index = Some(self.files.len() - 1);
                        // Use optimized selection update instead of full UI rebuild
                        self.update_file_selection(surface);
                    }
                }
                KeyCode::ArrowDown => {
                    if let Some(ref mut idx) = self.selected_file_index {
                        if *idx < self.files.len() - 1 {
                            *idx += 1;
                            // Use optimized selection update instead of full UI rebuild
                            self.update_file_selection(surface);
                        }
                    } else if !self.files.is_empty() {
                        self.selected_file_index = Some(0);
                        // Use optimized selection update instead of full UI rebuild
                        self.update_file_selection(surface);
                    }
                }
                KeyCode::Return => {
                    if let Some(idx) = self.selected_file_index {
                        if let Some(file) = self.files.get(idx).cloned() {
                            if file.is_directory {
                                // Navigate into directory
                                if let Ok(_) = self.navigate_into_directory(&file.name) {
                                    self.setup_ui(surface);
                                }
                            } else {
                                // Open file
                                self.mode = FileManagerMode::ViewFile(file);
                                self.setup_ui(surface);
                            }
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    // Clipboard operation handlers
    fn handle_copy_click(&mut self, surface: &mut Surface) -> bool {
        if let Some(idx) = self.selected_file_index {
            if let Some(file) = self.files.get(idx) {
                let file_path = if self.current_path == "/" {
                    format!("/{}", file.name)
                } else {
                    format!("{}/{}", self.current_path, file.name)
                };

                self.clipboard = Some(ClipboardEntry {
                    file_path: file_path.clone(),
                    file_name: file.name.clone(),
                    is_directory: file.is_directory,
                    operation: ClipboardOperation::Copy,
                });

                self.update_status_message(surface, format!("Copied '{}' to clipboard", file.name));
                // Refresh UI to update paste button state
                self.setup_ui(surface);
            }
        } else {
            self.update_status_message(
                surface,
                "Please select a file or folder to copy".to_string(),
            );
        }
        true
    }

    fn handle_cut_click(&mut self, surface: &mut Surface) -> bool {
        if let Some(idx) = self.selected_file_index {
            if let Some(file) = self.files.get(idx) {
                let file_path = if self.current_path == "/" {
                    format!("/{}", file.name)
                } else {
                    format!("{}/{}", self.current_path, file.name)
                };

                self.clipboard = Some(ClipboardEntry {
                    file_path: file_path.clone(),
                    file_name: file.name.clone(),
                    is_directory: file.is_directory,
                    operation: ClipboardOperation::Cut,
                });

                self.update_status_message(surface, format!("Cut '{}' to clipboard", file.name));
                // Refresh UI to update paste button state and show cut item differently
                self.setup_ui(surface);
            }
        } else {
            self.update_status_message(
                surface,
                "Please select a file or folder to cut".to_string(),
            );
        }
        true
    }

    fn handle_paste_click(&mut self, surface: &mut Surface) -> bool {
        if let Some(clipboard_entry) = &self.clipboard.clone() {
            let dest_path = if self.current_path == "/" {
                format!("/{}", clipboard_entry.file_name)
            } else {
                format!("{}/{}", self.current_path, clipboard_entry.file_name)
            };

            let result = match clipboard_entry.operation {
                ClipboardOperation::Copy => {
                    if clipboard_entry.is_directory {
                        copy_directory(&clipboard_entry.file_path, &dest_path)
                    } else {
                        copy_file(&clipboard_entry.file_path, &dest_path)
                    }
                }
                ClipboardOperation::Cut => move_item(&clipboard_entry.file_path, &dest_path),
            };

            match result {
                Ok(()) => {
                    let operation_name = match clipboard_entry.operation {
                        ClipboardOperation::Copy => "copied",
                        ClipboardOperation::Cut => "moved",
                    };

                    self.update_status_message(
                        surface,
                        format!(
                            "Successfully {} '{}'",
                            operation_name, clipboard_entry.file_name
                        ),
                    );

                    // Clear clipboard after successful cut operation
                    if matches!(clipboard_entry.operation, ClipboardOperation::Cut) {
                        self.clipboard = None;
                    }

                    self.refresh_file_list();
                    self.setup_ui(surface);
                }
                Err(e) => {
                    self.update_status_message(surface, format!("Paste failed: {}", e));
                }
            }
        } else {
            self.update_status_message(surface, "Clipboard is empty".to_string());
        }
        true
    }

    fn handle_rename_click_from_browse(&mut self, surface: &mut Surface) -> bool {
        if let Some(idx) = self.selected_file_index {
            if let Some(file) = self.files.get(idx).cloned() {
                self.mode = FileManagerMode::Rename(file.clone());
                self.input_text = file.name;
                self.setup_ui(surface);
            }
        } else {
            self.update_status_message(
                surface,
                "Please select a file or folder to rename".to_string(),
            );
        }
        true
    }

    fn handle_rename_click(&mut self, x: usize, y: usize, surface: &mut Surface) -> bool {
        if self.create_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN, surface.height - 60, 80, BUTTON_HEIGHT) {
                self.perform_rename(surface);
                return true;
            }
        }

        if self.back_btn_idx.is_some() {
            if self.is_button_clicked(x, y, MARGIN + 90, surface.height - 60, 80, BUTTON_HEIGHT) {
                self.mode = FileManagerMode::Browse;
                self.setup_ui(surface);
                return true;
            }
        }

        false
    }

    fn perform_rename(&mut self, surface: &mut Surface) {
        if self.input_text.trim().is_empty() {
            self.update_status_message(surface, "Please enter a new name".to_string());
            return;
        }

        if let FileManagerMode::Rename(file) = &self.mode {
            let old_path = if self.current_path == "/" {
                format!("/{}", file.name)
            } else {
                format!("{}/{}", self.current_path, file.name)
            };

            match rename_item(&old_path, &self.input_text) {
                Ok(()) => {
                    self.refresh_file_list();
                    self.mode = FileManagerMode::Browse;
                    self.input_text.clear();
                    self.setup_ui(surface);
                    self.update_status_message(
                        surface,
                        format!("Successfully renamed to '{}'", self.input_text),
                    );
                }
                Err(e) => {
                    self.update_status_message(surface, format!("Rename failed: {}", e));
                }
            }
        }
    }

    fn get_breadcrumb_text(&self) -> String {
        if self.current_path == "/" {
            "/ (Root)".to_string()
        } else {
            format!("{}", self.current_path)
        }
    }

    pub fn render(&mut self, _surface: &mut Surface) {
        // The UI is already set up, just make sure it's current
        // This could be extended to handle dynamic updates
    }
}
