use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use pc_keyboard::KeyCode;

use crate::{
    desktop::{
        calculator::Calculator, filemanager::FileManager, notepad::Notepad, sysinfo::SysInfo,
    },
    framebuffer::{Color, FrameBufferWriter},
    surface::{Rect, Surface},
};

pub struct DragCache {
    background_buffer: Vec<u8>,
    cached_bounds: Rect,
    is_valid: bool,
}

pub enum Application {
    Calculator(Calculator),
    FileManager(FileManager),
    Notepad(Notepad),
    SysInfo(SysInfo),
}

pub struct Window {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub id: usize,
    pub title: String,
    pub surface: Surface,
    pub dragging_offset: Option<(i16, i16)>,
    pub is_dragging: bool,
    pub drag_preview_x: usize,
    pub drag_preview_y: usize,
    drag_cache: Option<DragCache>,
    pub application: Option<Application>,
}

impl Window {
    pub fn new(
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        id: usize,
        title: String,
        application: Option<Application>,
    ) -> Self {
        let background_color = application.as_ref().map_or(Color::BLACK, |app| match app {
            Application::Calculator(_) => Color::GRAY,
            Application::FileManager(_) => Color::new(240, 240, 240),
            Application::Notepad(_) => Color::WHITE,
            Application::SysInfo(_) => Color::DARKGRAY,
        });
        let surface = Surface::new(width, height, background_color);

        Self {
            x,
            y,
            width,
            height,
            id,
            title,
            surface,
            application,
            dragging_offset: None,
            is_dragging: false,
            drag_preview_x: x,
            drag_preview_y: y,
            drag_cache: None,
        }
    }

    /// Get the window bounds including titlebar and border
    pub fn get_full_bounds(&self) -> Rect {
        Rect::new(
            self.x.saturating_sub(1),
            self.y.saturating_sub(20),
            self.width + 2,
            self.height + 21,
        )
    }

    /// Get the window content bounds (just the surface area)
    pub fn get_content_bounds(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.height)
    }

    /// Check if this window intersects with the given dirty regions
    pub fn intersects_dirty_regions(&self, dirty_regions: &[Rect]) -> bool {
        let window_bounds = self.get_full_bounds();
        dirty_regions
            .iter()
            .any(|rect| rect.intersects(&window_bounds))
    }

    pub fn render(&mut self, framebuffer: &mut FrameBufferWriter, force: bool) -> bool {
        match &mut self.application {
            Some(Application::Calculator(calculator)) => {
                calculator.render(&mut self.surface);
            }
            Some(Application::FileManager(filemanager)) => {
                filemanager.render(&mut self.surface);
            }
            Some(Application::Notepad(notepad)) => {
                self.title = notepad.get_title();

                notepad.render(&mut self.surface);
            }
            Some(Application::SysInfo(sysinfo)) => {
                sysinfo.render(&mut self.surface);
            }
            None => {}
        }

        return self.surface.render(framebuffer, self.x, self.y, force);
    }

    pub fn render_decorations(&self, framebuffer: &mut FrameBufferWriter) {
        // Window outline
        framebuffer.draw_rect_outline(
            (self.x - 1, self.y - 1),
            (self.x + self.width, self.y + self.height),
            Color::BLACK,
        );

        // Titlebar
        framebuffer.draw_rect(
            (self.x - 1, self.y - 20),
            (self.x + self.width, self.y),
            Color::BLACK,
        );
        framebuffer.draw_raw_text(
            &self.title,
            self.x + 5,
            self.y - 15,
            Color::WHITE,
            Color::BLACK,
            noto_sans_mono_bitmap::FontWeight::Regular,
            noto_sans_mono_bitmap::RasterHeight::Size16,
        );

        // Close button
        framebuffer.draw_rect(
            (self.x + self.width - 20, self.y - 20),
            (self.x + self.width, self.y),
            Color::RED,
        );
        framebuffer.draw_line(
            (self.x + self.width - 15, self.y - 15),
            (self.x + self.width - 5, self.y - 5),
            Color::WHITE,
        );
        framebuffer.draw_line(
            (self.x + self.width - 15, self.y - 5),
            (self.x + self.width - 5, self.y - 15),
            Color::WHITE,
        );
    }

    /// Get the drag preview outline bounds
    fn get_drag_preview_bounds(&self) -> Rect {
        Rect::new(
            self.drag_preview_x.saturating_sub(2),
            self.drag_preview_y.saturating_sub(21),
            self.width + 4,
            self.height + 23,
        )
    }

    /// Cache the background under the drag preview outline
    fn cache_background_under_outline(&mut self, framebuffer: &FrameBufferWriter) {
        let bounds = self.get_drag_preview_bounds();
        let bytes_per_pixel = 3; // Assuming RGB format for simplicity
        let buffer_size = (bounds.width + bounds.height) * bytes_per_pixel * 2;

        let mut background_buffer = Vec::with_capacity(buffer_size);

        for x in bounds.x..bounds.x + bounds.width {
            let color = framebuffer.read_pixel(x, bounds.y);
            background_buffer.extend_from_slice(&[color.r, color.g, color.b]);
        }
        for x in bounds.x..bounds.x + bounds.width {
            let color = framebuffer.read_pixel(x, bounds.y + bounds.height);
            background_buffer.extend_from_slice(&[color.r, color.g, color.b]);
        }

        for y in bounds.y..bounds.y + bounds.height {
            let color = framebuffer.read_pixel(bounds.x, y);
            background_buffer.extend_from_slice(&[color.r, color.g, color.b]);
        }
        for y in bounds.y..bounds.y + bounds.height {
            let color = framebuffer.read_pixel(bounds.x + bounds.width, y);
            background_buffer.extend_from_slice(&[color.r, color.g, color.b]);
        }

        self.drag_cache = Some(DragCache {
            background_buffer,
            cached_bounds: bounds,
            is_valid: true,
        });
    }

    /// Restore the cached background
    fn restore_cached_background(&mut self, framebuffer: &mut FrameBufferWriter) {
        if let Some(cache) = &self.drag_cache {
            if !cache.is_valid {
                return;
            }

            let bounds = &cache.cached_bounds;
            let bytes_per_pixel = 3;

            // Restore pixels row by row
            let mut buffer_idx = 0;
            for x in bounds.x..bounds.x + bounds.width {
                let r = cache.background_buffer[buffer_idx];
                let g = cache.background_buffer[buffer_idx + 1];
                let b = cache.background_buffer[buffer_idx + 2];
                framebuffer.write_pixel(x, bounds.y, Color::new(r, g, b));
                buffer_idx += bytes_per_pixel;
            }
            for x in bounds.x..bounds.x + bounds.width {
                let r = cache.background_buffer[buffer_idx];
                let g = cache.background_buffer[buffer_idx + 1];
                let b = cache.background_buffer[buffer_idx + 2];
                framebuffer.write_pixel(x, bounds.y + bounds.height, Color::new(r, g, b));
                buffer_idx += bytes_per_pixel;
            }

            for y in bounds.y..bounds.y + bounds.height {
                let r = cache.background_buffer[buffer_idx];
                let g = cache.background_buffer[buffer_idx + 1];
                let b = cache.background_buffer[buffer_idx + 2];
                framebuffer.write_pixel(bounds.x, y, Color::new(r, g, b));
                buffer_idx += bytes_per_pixel;
            }
            for y in bounds.y..bounds.y + bounds.height {
                let r = cache.background_buffer[buffer_idx];
                let g = cache.background_buffer[buffer_idx + 1];
                let b = cache.background_buffer[buffer_idx + 2];
                framebuffer.write_pixel(bounds.x + bounds.width, y, Color::new(r, g, b));
                buffer_idx += bytes_per_pixel;
            }
        }
    }

    /// Draw the drag preview outline
    fn draw_drag_outline(&self, framebuffer: &mut FrameBufferWriter) {
        let bounds = self.get_drag_preview_bounds();

        framebuffer.draw_rect_outline(
            (bounds.x, bounds.y),
            (bounds.x + bounds.width, bounds.y + bounds.height),
            Color::BLACK,
        );
    }

    /// Start dragging - enter drag mode
    pub fn start_drag(&mut self, framebuffer: &FrameBufferWriter) {
        self.is_dragging = true;
        self.drag_preview_x = self.x;
        self.drag_preview_y = self.y;
        self.cache_background_under_outline(framebuffer);
    }

    /// Update drag preview position
    pub fn update_drag_preview(
        &mut self,
        framebuffer: &mut FrameBufferWriter,
        new_x: usize,
        new_y: usize,
    ) {
        if !self.is_dragging {
            return;
        }

        // Restore background at old preview position
        self.restore_cached_background(framebuffer);

        // Update preview position
        self.drag_preview_x = new_x;
        self.drag_preview_y = new_y;

        // Cache background at new position and draw outline
        self.cache_background_under_outline(framebuffer);
        self.draw_drag_outline(framebuffer);
    }

    /// End dragging - commit to new position
    pub fn end_drag(&mut self, framebuffer: &mut FrameBufferWriter) -> (Rect, Rect) {
        if !self.is_dragging {
            return (self.get_full_bounds(), self.get_full_bounds());
        }

        // Get old bounds for dirty region
        let old_bounds = self.get_full_bounds();

        // Restore background at preview position
        self.restore_cached_background(framebuffer);

        // Update actual position to preview position
        self.x = self.drag_preview_x;
        self.y = self.drag_preview_y;

        // Get new bounds for dirty region
        let new_bounds = self.get_full_bounds();

        // Exit drag mode
        self.is_dragging = false;
        self.drag_cache = None;

        (old_bounds, new_bounds)
    }
}

pub struct WindowManager {
    pub windows: Vec<Window>,
    window_order: Vec<usize>, // Window IDs from back to front (last is topmost)
    next_window_id: usize,
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            window_order: Vec::new(),
            next_window_id: 1,
        }
    }

    pub fn add_window(&mut self, mut window: Window) {
        // Assign a unique ID to the window
        window.id = self.next_window_id;
        self.next_window_id += 1;

        match &mut window.application {
            Some(Application::Calculator(calculator)) => {
                calculator.init(&mut window.surface);
            }
            Some(Application::FileManager(filemanager)) => {
                filemanager.setup_ui(&mut window.surface);
            }
            Some(Application::Notepad(notepad)) => {
                notepad.init(&mut window.surface);
            }
            Some(Application::SysInfo(sysinfo)) => {
                sysinfo.init(&mut window.surface);
            }
            None => {}
        }

        // Add window to front of z-order (becomes focused)
        self.window_order.push(window.id);
        self.windows.push(window);
    }

    pub fn focus_window(&mut self, window_id: usize) {
        if self.windows.iter().any(|w| w.id == window_id) {
            // Remove window from current position in order
            self.window_order.retain(|&id| id != window_id);
            // Add to front (becomes focused)
            self.window_order.push(window_id);
        }

        // Mark window as dirty to ensure it gets re-rendered
        if let Some(window) = self.windows.iter_mut().find(|w| w.id == window_id) {
            window.surface.is_dirty = true;
        }
    }

    pub fn get_focused_window_id(&self) -> Option<usize> {
        self.window_order.last().copied()
    }

    pub fn get_windows_in_render_order(&self) -> Vec<&Window> {
        let mut windows: Vec<&Window> = Vec::new();

        // Add windows in z-order (back to front)
        for &window_id in &self.window_order {
            if let Some(window) = self.windows.iter().find(|w| w.id == window_id) {
                windows.push(window);
            }
        }

        windows
    }

    pub fn get_taskbar_windows(&self) -> Vec<(usize, &str, bool)> {
        let focused_id = self.get_focused_window_id();

        self.windows
            .iter()
            .map(|w| {
                let is_focused = Some(w.id) == focused_id;
                (w.id, w.title.as_str(), is_focused)
            })
            .collect()
    }

    pub fn render(
        &mut self,
        framebuffer: &mut FrameBufferWriter,
        desktop_dirty_regions: &[Rect],
    ) -> bool {
        let mut did_render = false;

        // Get windows in render order (focused window renders last/on top)
        let window_ids: Vec<usize> = self
            .get_windows_in_render_order()
            .iter()
            .map(|w| w.id)
            .collect();

        for window_id in window_ids {
            let window = self.windows.iter_mut().find(|w| w.id == window_id).unwrap();

            // Skip rendering if window is being dragged (only show drag preview)
            if window.is_dragging {
                continue;
            }

            // Only render window if it intersects with dirty regions or window itself is dirty
            let intersects_dirty = window.intersects_dirty_regions(desktop_dirty_regions);
            let should_render = window.surface.is_dirty || intersects_dirty;

            if window.render(framebuffer, should_render) {
                did_render = true;
            }

            if did_render {
                // Always render decorations when we render the window
                window.render_decorations(framebuffer);
            }
        }

        did_render
    }

    /// Handles mouse click events on windows.
    /// Returns: (handled, dirty_region)
    pub fn handle_mouse_click(
        &mut self,
        x: i16,
        y: i16,
    ) -> (bool, Option<(usize, usize, usize, usize)>) {
        let mut window_to_focus: Option<usize> = None;
        let mut window_to_remove: Option<usize> = None;
        let mut remove_bounds: Option<(usize, usize, usize, usize)> = None;
        let mut app_result: Option<(String, String)> = None;

        // Check if the click was on the close button first
        for window in &self.windows {
            if x as usize >= window.x + window.width - 20
                && x as usize <= window.x + window.width
                && y as usize >= window.y - 20
                && y as usize <= window.y
            {
                window_to_remove = Some(window.id);
                remove_bounds = Some((
                    window.x - 1,
                    window.y - 20,
                    window.width + 2,
                    window.height + 21,
                ));
                break;
            }
        }

        if let Some(window_id) = window_to_remove {
            self.windows.retain(|w| w.id != window_id);
            self.window_order.retain(|&id| id != window_id);

            return (true, remove_bounds);
        }

        // Check for clicks on window areas
        for window in &mut self.windows {
            // Check for clicks on title bar (for focusing)
            if x as usize >= window.x - 1
                && x as usize <= window.x + window.width
                && y as usize >= window.y - 20
                && y as usize <= window.y
            {
                window_to_focus = Some(window.id);
            }

            // Check for clicks on window content
            if x as usize >= window.x
                && x as usize <= window.x + window.width
                && y as usize >= window.y
                && y as usize <= window.y + window.height
            {
                window_to_focus = Some(window.id);

                if let Some(Application::Calculator(calculator)) = &mut window.application {
                    let x = (x as usize).saturating_sub(window.x);
                    let y = (y as usize).saturating_sub(window.y);

                    calculator.handle_mouse_click(x, y);
                    break;
                }
                if let Some(Application::Notepad(notepad)) = &mut window.application {
                    let x = (x as usize).saturating_sub(window.x);
                    let y = (y as usize).saturating_sub(window.y);

                    notepad.handle_mouse_click(x, y);
                    break;
                }
                if let Some(Application::FileManager(filemanager)) = &mut window.application {
                    let x = (x as usize).saturating_sub(window.x);
                    let y = (y as usize).saturating_sub(window.y);

                    let (_, open_app) = filemanager.handle_click(x, y, &mut window.surface);
                    if let Some((file_path, app)) = open_app {
                        app_result = Some((file_path, app));
                    }
                    break;
                }
                if let Some(Application::SysInfo(sysinfo)) = &mut window.application {
                    let x = (x as usize).saturating_sub(window.x);
                    let y = (y as usize).saturating_sub(window.y);

                    sysinfo.handle_mouse_click(x, y);
                    break;
                }
                break; // Always break after handling content click
            }
        }

        if let Some(window_id) = window_to_focus {
            self.focus_window(window_id);
        }

        if let Some((file_path, app)) = app_result {
            self.open_app_handler(file_path, app);
        }

        (window_to_focus.is_some(), None)
    }

    pub fn handle_taskbar_click(&mut self, window_id: usize) -> bool {
        if self.windows.iter().any(|w| w.id == window_id) {
            self.focus_window(window_id);
            true
        } else {
            false
        }
    }

    fn open_app_handler(&mut self, file_path: String, app: String) {
        match app.as_str() {
            "notepad" => launch_notepad_with_file(self, file_path),
            "calculator" => launch_calculator(self), // Who tf opens his files in calculator?!
            _ => {}
        }
    }

    pub fn handle_mouse_down(&mut self, x: i16, y: i16, framebuffer: &FrameBufferWriter) {
        for window in &mut self.windows {
            if x as usize >= window.x
                && x as usize <= window.x + window.width - 20
                && y as usize >= window.y - 20
                && y as usize <= window.y
            {
                window.dragging_offset = Some((x, y));
                window.start_drag(framebuffer);

                return;
            }
        }
    }

    pub fn handle_mouse_move(&mut self, x: i16, y: i16, framebuffer: &mut FrameBufferWriter) {
        for window in &mut self.windows {
            if let Some(offset) = window.dragging_offset {
                let delta_x = x - offset.0;
                let delta_y = y - offset.1;

                window.dragging_offset = Some((x, y));

                // Calculate new position
                let new_x = (window.drag_preview_x as i16)
                    .saturating_add(delta_x)
                    .max(1) as usize;
                let new_y = (window.drag_preview_y as i16)
                    .saturating_add(delta_y)
                    .max(20) as usize;

                // Update drag preview
                window.update_drag_preview(framebuffer, new_x, new_y);

                return;
            }
        }
    }

    pub fn handle_mouse_release(
        &mut self,
        framebuffer: &mut FrameBufferWriter,
    ) -> Vec<(usize, usize, usize, usize)> {
        let mut dirty_regions = Vec::new();

        for window in &mut self.windows {
            if window.dragging_offset.is_some() {
                window.dragging_offset = None;

                // End drag and get dirty regions
                let (old_bounds, new_bounds) = window.end_drag(framebuffer);

                // Add both old and new positions as dirty regions
                dirty_regions.push((
                    old_bounds.x,
                    old_bounds.y,
                    old_bounds.width,
                    old_bounds.height,
                ));
                if old_bounds != new_bounds {
                    dirty_regions.push((
                        new_bounds.x,
                        new_bounds.y,
                        new_bounds.width,
                        new_bounds.height,
                    ));
                }
            }
        }

        dirty_regions
    }

    pub fn handle_char_input(
        &mut self,
        ch: char,
        ctrl_pressed: bool,
        _alt_pressed: bool,
        _shift_pressed: bool,
    ) {
        // Send character input only to the focused window
        if let Some(focused_id) = self.get_focused_window_id() {
            if let Some(window) = self.windows.iter_mut().find(|w| w.id == focused_id) {
                match &mut window.application {
                    Some(Application::Notepad(notepad)) => {
                        notepad.handle_char_input(ch, ctrl_pressed);
                    }
                    Some(Application::FileManager(filemanager)) => {
                        filemanager.handle_char_input(ch, &mut window.surface);
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn handle_key_input(&mut self, key: KeyCode) {
        // Handle key input only for the focused window
        if let Some(focused_id) = self.get_focused_window_id() {
            if let Some(window) = self.windows.iter_mut().find(|w| w.id == focused_id) {
                match &mut window.application {
                    Some(Application::Notepad(notepad)) => {
                        notepad.handle_key_input(key);
                    }
                    Some(Application::FileManager(filemanager)) => {
                        filemanager.handle_key_input(key, &mut window.surface);
                    }
                    _ => {}
                }
            }
        }
    }
}

pub fn launch_calculator(window_manager: &mut WindowManager) {
    window_manager.add_window(Window::new(
        100,
        100,
        205,
        315,
        0, // Will be overridden by add_window
        "Calculator".to_string(),
        Some(Application::Calculator(Calculator::new())),
    ));
}

pub fn launch_filemanager(window_manager: &mut WindowManager) {
    window_manager.add_window(Window::new(
        120,
        80,
        500,
        400,
        0, // Will be overridden by add_window
        "File Manager".to_string(),
        Some(Application::FileManager(FileManager::new())),
    ));
}

pub fn launch_notepad(window_manager: &mut WindowManager) {
    window_manager.add_window(Window::new(
        150,
        150,
        600,
        400,
        0, // Will be overridden by add_window
        "Notepad".to_string(),
        Some(Application::Notepad(Notepad::new(None))),
    ));
}

pub fn launch_notepad_with_file(window_manager: &mut WindowManager, file_path: String) {
    window_manager.add_window(Window::new(
        150,
        150,
        600,
        400,
        0, // Will be overridden by add_window
        "Notepad".to_string(),
        Some(Application::Notepad(Notepad::new(Some(file_path)))),
    ));
}

pub fn launch_sysinfo(window_manager: &mut WindowManager) {
    window_manager.add_window(Window::new(
        200,
        100,
        400,
        350,
        0, // Will be overridden by add_window
        "System Information".to_string(),
        Some(Application::SysInfo(SysInfo::new())),
    ));
}
