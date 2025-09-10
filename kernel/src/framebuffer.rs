use alloc::vec;
use alloc::vec::Vec;
use bootloader_api::info::{FrameBuffer, FrameBufferInfo, PixelFormat};
use conquer_once::spin::OnceCell;
use core::{fmt, slice};
use font_constants::BACKUP_CHAR;
use noto_sans_mono_bitmap::{
    FontWeight, RasterHeight, RasterizedChar, get_raster, get_raster_width,
};
use spinning_top::Spinlock;
use x86_64::{
    VirtAddr,
    structures::paging::{
        FrameAllocator, Mapper, Page, PageTableFlags, PhysFrame, Size2MiB, Size4KiB,
        mapper::MapToError,
    },
};

use crate::serial_println;

pub static FRAMEBUFFER: OnceCell<Spinlock<FrameBufferWriter>> = OnceCell::uninit();
pub static SCREEN_SIZE: OnceCell<(u16, u16)> = OnceCell::uninit();

/// Prints to framebuffer
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::framebuffer::_print(format_args!($($arg)*))
    };
}

/// Prints to framebuffer, appending a newline.
#[macro_export]
macro_rules! println {
    () => ($crate::serial_print!("\n"));
    ($fmt:expr) => ($crate::print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::print!(
        concat!($fmt, "\n"), $($arg)*));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    use x86_64::instructions::interrupts;
    interrupts::without_interrupts(|| {
        if let Some(fb) = FRAMEBUFFER.get() {
            if let Some(mut guard) = fb.try_lock() {
                guard.write_fmt(args).unwrap();
            } else {
                serial_println!("[FB DEADLOCK] - {}", args.as_str().unwrap_or("<unknown>"));
            }
        }
    });
}

/// Additional vertical space between lines
const LINE_SPACING: usize = 2;
/// Additional horizontal space between characters.
const LETTER_SPACING: usize = 0;

/// Padding from the border. Prevent that font is too close to border.
const BORDER_PADDING: usize = 1;

const fn calculate_cursor_bg_data_size(rows: &[(isize, isize)]) -> usize {
    let mut sum = 0;
    let mut i = 0;

    while i < rows.len() {
        // For some weird reason rust doesn't allow iter, map or for
        let (start_x, end_x) = rows[i];
        sum += (end_x - start_x + 1) as usize;
        i += 1;
    }
    sum
}

const CURSOR_ROWS: [(isize, isize); 17] = [
    (0, 0), // Tip of the triangle
    (0, 1),
    (0, 3),
    (0, 4),
    (1, 6),
    (1, 7),
    (1, 9),
    (1, 10),
    (1, 11),
    (2, 13),
    (2, 14),
    (2, 13),
    (2, 11),
    (2, 9),
    (2, 7),
    (3, 5),
    (3, 3),
];

const CURSOR_BG_DATA_SIZE: usize = calculate_cursor_bg_data_size(&CURSOR_ROWS);
const CURSOR_ROW_OFFSET: usize = 0; // Offset for the cursor rows from the top of the cursor
const CURSOR_COLOR: Color = Color::BLUE;

const TEXT_COLOR: Color = Color::new(255, 255, 150);

/// Tile size for dirty rectangle tracking
const TILE_SIZE: usize = 20;

/// Dirty rectangle tracking for efficient rendering
pub struct DirtyTracker {
    tiles_width: usize,
    tiles_height: usize,
    dirty_tiles: Vec<bool>,
    screen_width: usize,
    screen_height: usize,
}

impl DirtyTracker {
    pub fn new(screen_width: usize, screen_height: usize) -> Self {
        let tiles_width = (screen_width + TILE_SIZE - 1) / TILE_SIZE;
        let tiles_height = (screen_height + TILE_SIZE - 1) / TILE_SIZE;
        let total_tiles = tiles_width * tiles_height;

        serial_println!(
            "Framebuffer initialized with {}x{} tiles",
            tiles_width,
            tiles_height
        );

        Self {
            tiles_width,
            tiles_height,
            dirty_tiles: vec![false; total_tiles],
            screen_width,
            screen_height,
        }
    }

    /// Mark a pixel region as dirty
    pub fn mark_dirty(&mut self, x: usize, y: usize, width: usize, height: usize) {
        let start_tile_x = x / TILE_SIZE;
        let start_tile_y = y / TILE_SIZE;
        let end_tile_x = ((x + width).min(self.screen_width) + TILE_SIZE - 1) / TILE_SIZE;
        let end_tile_y = ((y + height).min(self.screen_height) + TILE_SIZE - 1) / TILE_SIZE;

        for tile_y in start_tile_y..end_tile_y.min(self.tiles_height) {
            for tile_x in start_tile_x..end_tile_x.min(self.tiles_width) {
                let tile_index = tile_y * self.tiles_width + tile_x;
                if tile_index < self.dirty_tiles.len() {
                    self.dirty_tiles[tile_index] = true;
                }
            }
        }
    }

    /// Mark a single pixel as dirty
    pub fn mark_pixel_dirty(&mut self, x: usize, y: usize) {
        if x >= self.screen_width || y >= self.screen_height {
            return;
        }

        let tile_x = x / TILE_SIZE;
        let tile_y = y / TILE_SIZE;
        let tile_index = tile_y * self.tiles_width + tile_x;

        if tile_index < self.dirty_tiles.len() {
            self.dirty_tiles[tile_index] = true;
        }
    }

    /// Get all dirty tile regions and clear them
    pub fn get_dirty_regions(&mut self) -> Vec<(u8, u8)> {
        let mut regions = Vec::new();

        for tile_y in 0..self.tiles_height {
            for tile_x in 0..self.tiles_width {
                let tile_index = tile_y * self.tiles_width + tile_x;
                if tile_index < self.dirty_tiles.len() && self.dirty_tiles[tile_index] {
                    regions.push((tile_x as u8, tile_y as u8));

                    self.dirty_tiles[tile_index] = false; // Clear the dirty flag
                }
            }
        }

        regions
    }

    /// Mark all tiles as dirty (force full redraw)
    pub fn mark_all_dirty(&mut self) {
        self.dirty_tiles.fill(true);
    }

    /// Clear all dirty flags without returning regions
    pub fn clear_all_dirty(&mut self) {
        self.dirty_tiles.fill(false);
    }

    /// Check if any tiles are dirty
    pub fn has_dirty_tiles(&self) -> bool {
        self.dirty_tiles.iter().any(|&dirty| dirty)
    }
}

/// Constants for the usage of the [`noto_sans_mono_bitmap`] crate.
mod font_constants {
    use super::*;

    /// Height of each char raster. The font size is ~0.84% of this. Thus, this is the line height that
    /// enables multiple characters to be side-by-side and appear optically in one line in a natural way.
    pub const CHAR_RASTER_HEIGHT: RasterHeight = RasterHeight::Size16;

    /// The width of each single symbol of the mono space font.
    pub const CHAR_RASTER_WIDTH: usize = get_raster_width(FontWeight::Regular, CHAR_RASTER_HEIGHT);

    /// Backup character if a desired symbol is not available by the font.
    /// The '�' character requires the feature "unicode-specials".
    pub const BACKUP_CHAR: char = '�';

    pub const FONT_WEIGHT: FontWeight = FontWeight::Regular;
}

/// Returns the raster of the given char or the raster of [`font_constants::BACKUP_CHAR`].
fn get_char_raster(c: char, font_weight: FontWeight, font_size: RasterHeight) -> RasterizedChar {
    fn get(c: char, font_weight: FontWeight, font_size: RasterHeight) -> Option<RasterizedChar> {
        get_raster(c, font_weight, font_size)
    }
    get(c, font_weight, font_size).unwrap_or_else(|| {
        get(BACKUP_CHAR, font_weight, font_size).expect("Should get raster of backup char.")
    })
}

#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const DARKGRAY: Color = Color {
        r: 50,
        g: 50,
        b: 50,
    };
    pub const GRAY: Color = Color {
        r: 150,
        g: 150,
        b: 150,
    };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
    };
    pub const RED: Color = Color { r: 255, g: 0, b: 0 };
    pub const GREEN: Color = Color { r: 0, g: 255, b: 0 };
    pub const BLUE: Color = Color { r: 0, g: 0, b: 255 };

    pub fn to_u8(&self) -> u8 {
        (self.r + self.g + self.b) / 3
    }

    pub fn to_rgb(&self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }

    pub fn to_bgr(&self) -> [u8; 3] {
        [self.b, self.g, self.r]
    }
}

/// BackBuffer using 2MiB memory pages for efficient double buffering
pub struct BackBuffer {
    buffer: &'static mut [u8],
    frames: Vec<PhysFrame<Size2MiB>>,
    info: FrameBufferInfo,
    virtual_addr: VirtAddr,
    total_size: usize,
}

impl BackBuffer {
    /// Create a new backbuffer using 2MiB pages
    pub fn new(
        info: FrameBufferInfo,
        mapper: &mut impl Mapper<Size2MiB>,
        frame_allocator: &mut (impl FrameAllocator<Size2MiB> + FrameAllocator<Size4KiB>),
    ) -> Result<Self, MapToError<Size2MiB>> {
        // Calculate required buffer size
        let buffer_size = info.height * info.stride * info.bytes_per_pixel;
        serial_println!("BackBuffer: Required size {} bytes", buffer_size);

        // Calculate how many 2MiB pages we need
        const PAGE_SIZE_2MIB: usize = 2 * 1024 * 1024;
        let num_pages = (buffer_size + PAGE_SIZE_2MIB - 1) / PAGE_SIZE_2MIB; // Round up
        let total_allocated_size = num_pages * PAGE_SIZE_2MIB;

        serial_println!("BackBuffer: Need {} pages of 2MiB each", num_pages);

        // Allocate multiple 2MiB frames
        let mut frames = Vec::new();
        for i in 0..num_pages {
            let frame = frame_allocator
                .allocate_frame()
                .ok_or(MapToError::FrameAllocationFailed)?;

            serial_println!(
                "BackBuffer: Allocated 2MiB frame {} at {:?}",
                i,
                frame.start_address()
            );
            frames.push(frame);
        }

        // Use a high virtual address space for backbuffer (avoid conflicts)
        let virtual_addr = VirtAddr::new(0xFFFF_8000_0000_0000u64);
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;

        // Map all frames to consecutive virtual pages
        for (i, frame) in frames.iter().enumerate() {
            let page_addr = virtual_addr + (i * PAGE_SIZE_2MIB) as u64;
            let page = Page::<Size2MiB>::containing_address(page_addr);

            unsafe {
                mapper.map_to(page, *frame, flags, frame_allocator)?.flush();
            }

            serial_println!(
                "BackBuffer: Mapped frame {} to virtual address {:?}",
                i,
                page_addr
            );
        }

        // Create a slice over the mapped memory
        let buffer = unsafe {
            slice::from_raw_parts_mut(virtual_addr.as_mut_ptr::<u8>(), total_allocated_size)
        };

        // Clear the buffer
        buffer.fill(0);

        serial_println!(
            "BackBuffer: Successfully created at virtual address {:?} with {} bytes",
            virtual_addr,
            total_allocated_size
        );

        Ok(BackBuffer {
            buffer,
            frames,
            info,
            virtual_addr,
            total_size: total_allocated_size,
        })
    }

    /// Get a mutable slice to the buffer
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        // Only return the actual framebuffer size, not the full allocated size
        let actual_size = self.info.height * self.info.stride * self.info.bytes_per_pixel;
        let actual_size = actual_size.min(self.total_size);
        &mut self.buffer[..actual_size]
    }

    /// Get an immutable slice to the buffer
    pub fn buffer(&self) -> &[u8] {
        let actual_size = self.info.height * self.info.stride * self.info.bytes_per_pixel;
        let actual_size = actual_size.min(self.total_size);
        &self.buffer[..actual_size]
    }

    /// Get buffer info
    pub fn info(&self) -> &FrameBufferInfo {
        &self.info
    }

    /// Clear the backbuffer
    pub fn clear(&mut self) {
        self.buffer_mut().fill(0);
    }

    /// Copy from another buffer to this backbuffer
    pub fn copy_from(&mut self, src: &[u8]) {
        let dst = self.buffer_mut();
        let copy_size = src.len().min(dst.len());
        dst[..copy_size].copy_from_slice(&src[..copy_size]);
    }

    /// Get the physical frames for this backbuffer
    pub fn frames(&self) -> &[PhysFrame<Size2MiB>] {
        &self.frames
    }

    /// Get the first physical frame for this backbuffer (for compatibility)
    pub fn frame(&self) -> PhysFrame<Size2MiB> {
        self.frames[0]
    }

    /// Get the virtual address of this backbuffer
    pub fn virtual_addr(&self) -> VirtAddr {
        self.virtual_addr
    }
}

struct CursorBackground {
    saved_pixels: [u8; CURSOR_BG_DATA_SIZE * 4],
    previous_pos: Option<(usize, usize)>,
}

impl CursorBackground {
    fn new(bytes_per_pixel: usize) -> Self {
        if bytes_per_pixel > 4 {
            serial_println!("Cursor background only supports 4 bytes per pixel (RGB).");
            panic!("Cursor background only supports 4 bytes per pixel (RGB).");
        }

        Self {
            saved_pixels: [0; CURSOR_BG_DATA_SIZE * 4],
            previous_pos: None,
        }
    }
}

/// Allows logging text to a pixel-based framebuffer.
pub struct FrameBufferWriter {
    framebuffer: &'static mut [u8],
    info: FrameBufferInfo,
    x_pos: usize,
    y_pos: usize,
    cursor_background: CursorBackground,
    backbuffer: BackBuffer,
    dirty_tracker: DirtyTracker,
}

impl FrameBufferWriter {
    /// Creates a new logger that uses the given framebuffer.
    pub fn new(
        framebuffer: &'static mut [u8],
        info: FrameBufferInfo,
        frame_allocator: &mut (impl FrameAllocator<Size2MiB> + FrameAllocator<Size4KiB>),
        mapper: &mut impl Mapper<Size2MiB>,
    ) -> Self {
        let mut logger = Self {
            framebuffer,
            info,
            x_pos: 0,
            y_pos: 0,
            cursor_background: CursorBackground::new(info.bytes_per_pixel),
            backbuffer: BackBuffer::new(info, mapper, frame_allocator)
                .expect("Failed to create backbuffer"),
            dirty_tracker: DirtyTracker::new(info.width, info.height),
        };

        logger.clear();
        logger
    }

    fn newline(&mut self) {
        self.y_pos += font_constants::CHAR_RASTER_HEIGHT.val() + LINE_SPACING;
        self.carriage_return()
    }

    fn carriage_return(&mut self) {
        self.x_pos = BORDER_PADDING;
    }

    pub fn fill(&mut self, brightness: u8) {
        // Mark all tiles as dirty since we're filling the entire buffer
        self.dirty_tracker.mark_all_dirty();

        self.backbuffer.buffer_mut().fill(brightness);
    }

    /// Erases all text on the screen. Resets `self.x_pos` and `self.y_pos`.
    pub fn clear(&mut self) {
        self.x_pos = BORDER_PADDING;
        self.y_pos = BORDER_PADDING;

        // Mark all tiles as dirty since we're clearing the entire buffer
        self.dirty_tracker.mark_all_dirty();

        self.backbuffer.clear();
    }

    fn width(&self) -> usize {
        self.info.width
    }

    fn height(&self) -> usize {
        self.info.height
    }

    pub fn size(&self) -> (usize, usize) {
        (self.info.width, self.info.height)
    }

    /// Writes a single char to the framebuffer. Takes care of special control characters, such as
    /// newlines and carriage returns.
    fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.carriage_return(),
            c => {
                let new_xpos = self.x_pos + font_constants::CHAR_RASTER_WIDTH;
                if new_xpos >= self.width() {
                    self.newline();
                }
                let new_ypos =
                    self.y_pos + font_constants::CHAR_RASTER_HEIGHT.val() + BORDER_PADDING;
                if new_ypos >= self.height() {
                    self.clear();
                }
                self.write_rendered_char(get_char_raster(
                    c,
                    font_constants::FONT_WEIGHT,
                    font_constants::CHAR_RASTER_HEIGHT,
                ));
            }
        }
    }

    /// Prints a rendered char into the framebuffer.
    /// Updates `self.x_pos`.
    fn write_rendered_char(&mut self, rendered_char: RasterizedChar) {
        self.write_rendered_char_at_pos(
            self.x_pos,
            self.y_pos,
            &rendered_char,
            TEXT_COLOR,
            Color::BLACK,
        );
        self.x_pos += rendered_char.width() + LETTER_SPACING;
    }

    fn write_rendered_char_at_pos(
        &mut self,
        pos_x: usize,
        pos_y: usize,
        rendered_char: &RasterizedChar,
        color: Color,
        bg_color: Color,
    ) {
        for (y, row) in rendered_char.raster().iter().enumerate() {
            for (x, byte) in row.iter().enumerate() {
                if byte == &0 {
                    continue;
                }

                let byte = *byte as f32 / 255.0;
                let byte_reverse = 1.0 - byte;

                let color = Color::new(
                    ((color.r as f32 * byte) + (bg_color.r as f32 * byte_reverse)) as u8,
                    ((color.g as f32 * byte) + (bg_color.g as f32 * byte_reverse)) as u8,
                    ((color.b as f32 * byte) + (bg_color.b as f32 * byte_reverse)) as u8,
                );

                self.write_pixel(pos_x + x, pos_y + y, color);
            }
        }
    }

    pub fn write_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x >= self.width() || y >= self.height() {
            return; // Out of bounds
        }

        // Mark tile as dirty
        self.dirty_tracker.mark_pixel_dirty(x, y);

        let pixel_offset = y * self.info.stride + x;
        let color = match self.info.pixel_format {
            PixelFormat::Rgb => [color.r, color.g, color.b, 0],
            PixelFormat::Bgr => [color.b, color.g, color.r, 0],
            PixelFormat::U8 => [if color.to_u8() > 200 { 0xf } else { 0 }, 0, 0, 0],
            other => {
                // set a supported (but invalid) pixel format before panicking to avoid a double
                // panic; it might not be readable though
                self.info.pixel_format = PixelFormat::Rgb;
                panic!("pixel format {:?} not supported in logger", other)
            }
        };

        let bytes_per_pixel = self.info.bytes_per_pixel;
        let byte_offset = pixel_offset * bytes_per_pixel;

        let bb_slice = self.backbuffer.buffer_mut();
        if byte_offset + bytes_per_pixel <= bb_slice.len() {
            bb_slice[byte_offset..(byte_offset + bytes_per_pixel)]
                .copy_from_slice(&color[..bytes_per_pixel]);
        }
    }

    pub fn read_pixel(&self, x: usize, y: usize) -> Color {
        if x >= self.width() || y >= self.height() {
            return Color::BLACK; // Out of bounds, return black
        }

        let pixel_offset = y * self.info.stride + x;
        let bytes_per_pixel = self.info.bytes_per_pixel;
        let byte_offset = pixel_offset * bytes_per_pixel;

        let bb_slice = self.backbuffer.buffer();
        if byte_offset + bytes_per_pixel <= bb_slice.len() {
            let bytes = &bb_slice[byte_offset..(byte_offset + bytes_per_pixel)];
            return match self.info.pixel_format {
                PixelFormat::Rgb => Color::new(bytes[0], bytes[1], bytes[2]),
                PixelFormat::Bgr => Color::new(bytes[2], bytes[1], bytes[0]),
                PixelFormat::U8 => {
                    let intensity = if bytes[0] > 0 { 255 } else { 0 };
                    Color::new(intensity, intensity, intensity)
                }
                other => {
                    panic!("pixel format {:?} not supported for reading", other)
                }
            };
        } else {
            return Color::BLACK; // Out of bounds, return black
        }
    }

    pub fn write_raw_pixel_row(&mut self, x: usize, y: usize, data: &[u8]) {
        if y >= self.height() || x >= self.width() {
            return;
        }

        let max_width = (self.width() - x).min(data.len() / self.info.bytes_per_pixel);
        let bytes_per_pixel = self.info.bytes_per_pixel;
        let start_offset = (y * self.info.stride + x) * bytes_per_pixel;
        let copy_size = max_width * bytes_per_pixel;

        // Mark affected region as dirty
        if max_width > 0 {
            self.dirty_tracker.mark_dirty(x, y, max_width, 1);
        }

        let bb_slice = self.backbuffer.buffer_mut();
        if start_offset + copy_size <= bb_slice.len() {
            bb_slice[start_offset..start_offset + copy_size].copy_from_slice(&data[..copy_size]);
        }
    }

    /// Read a horizontal line of pixels at once
    /// Note: Returns a temporary buffer when reading from backbuffer
    pub fn read_raw_pixel_row(&self, x: usize, y: usize, count: usize) -> Vec<u8> {
        if y >= self.height() || x >= self.width() {
            return Vec::new();
        }

        let max_width = (self.width() - x).min(count);
        let bytes_per_pixel = self.info.bytes_per_pixel;
        let start_offset = (y * self.info.stride + x) * bytes_per_pixel;
        let data_size = max_width * bytes_per_pixel;

        let bb_slice = self.backbuffer.buffer();
        if start_offset + data_size <= bb_slice.len() {
            return bb_slice[start_offset..start_offset + data_size].to_vec();
        }

        return Vec::new();
    }

    pub fn draw_mouse_cursor(&mut self, x: usize, y: usize) {
        // Mark previous cursor position as dirty if it exists
        if let Some(prev_pos) = self.cursor_background.previous_pos {
            let prev_bounds = Self::get_cursor_bounds(prev_pos.0, prev_pos.1);
            self.dirty_tracker.mark_dirty(
                prev_bounds.0,
                prev_bounds.1,
                prev_bounds.2,
                prev_bounds.3,
            );
            self.restore_cursor_background(prev_pos);
        }

        // Mark new cursor position as dirty
        let cursor_bounds = Self::get_cursor_bounds(x, y);
        self.dirty_tracker.mark_dirty(
            cursor_bounds.0,
            cursor_bounds.1,
            cursor_bounds.2,
            cursor_bounds.3,
        );

        self.save_cursor_background(x, y);
        self.draw_cursor(x, y);

        self.cursor_background.previous_pos = Some((x, y));
    }

    /// Get the bounds of the mouse cursor at the given position
    pub fn get_cursor_bounds(x: usize, y: usize) -> (usize, usize, usize, usize) {
        let start_y = y.saturating_sub(CURSOR_ROW_OFFSET);
        let mut min_x = x;
        let mut max_x = x;
        let mut max_y = start_y;

        for (i, (start_x, end_x)) in CURSOR_ROWS.iter().enumerate() {
            let cursor_y = start_y + i;
            let cursor_start_x = (x as isize + start_x).max(0) as usize;
            let cursor_end_x = (x as isize + end_x).max(0) as usize;

            min_x = min_x.min(cursor_start_x);
            max_x = max_x.max(cursor_end_x);
            max_y = max_y.max(cursor_y);
        }

        (min_x, start_y, max_x - min_x + 1, max_y - start_y + 1)
    }

    /// Get the previous cursor position
    pub fn get_previous_cursor_pos(&self) -> Option<(usize, usize)> {
        self.cursor_background.previous_pos
    }

    fn draw_cursor(&mut self, x: usize, y: usize) {
        let start_y: usize = y.saturating_sub(CURSOR_ROW_OFFSET);

        for (i, (start_x, end_x)) in CURSOR_ROWS.iter().enumerate() {
            let cursor_y = start_y as isize + i as isize;
            let start_x = x as isize + start_x;
            let end_x = x as isize + end_x;

            if cursor_y < 0 || cursor_y >= self.height() as isize {
                continue; // Skip out-of-bounds rows
            }

            self.write_pixel_row(
                start_x as usize,
                end_x as usize,
                cursor_y as usize,
                &CURSOR_COLOR,
            );
        }
    }

    pub fn save_cursor_background(&mut self, x: usize, y: usize) {
        let start_y: usize = y.saturating_sub(CURSOR_ROW_OFFSET);

        let mut idx: usize = 0; // Index for the saved pixels
        for (i, (start_x, end_x)) in CURSOR_ROWS.iter().enumerate() {
            let cursor_y = start_y as isize + i as isize;

            let count = (end_x - start_x + 1)
                .min(self.width() as isize - x as isize - start_x)
                .max(0) as usize;

            let start_x = x as isize + start_x;

            let bytes_per_pixel = self.info.bytes_per_pixel;

            if cursor_y < 0 || cursor_y >= self.height() as isize {
                // Fill with zeros for out-of-bounds pixels
                let size = count * bytes_per_pixel;
                if idx + size <= self.cursor_background.saved_pixels.len() {
                    self.cursor_background.saved_pixels[idx..idx + size].fill(0);
                    idx += size;
                }
                continue;
            }

            if start_x < 0 || start_x > self.width() as isize {
                // Fill with zeros for out-of-bounds pixels
                let size = count * bytes_per_pixel;
                if idx + size <= self.cursor_background.saved_pixels.len() {
                    self.cursor_background.saved_pixels[idx..idx + size].fill(0);
                    idx += size;
                }
                continue;
            }

            // Read from active buffer (backbuffer or framebuffer)
            let copy_size = count * bytes_per_pixel;

            if idx + copy_size <= self.cursor_background.saved_pixels.len() {
                // Calculate the pixel offset directly to avoid borrow checker issues
                let pixel_offset = cursor_y as usize * self.info.stride + start_x as usize;
                let byte_offset = pixel_offset * self.info.bytes_per_pixel;
                let max_bytes = (self.width() - start_x as usize) * self.info.bytes_per_pixel;
                let actual_copy_size = copy_size.min(max_bytes);

                let bb_slice = self.backbuffer.buffer();
                if byte_offset + actual_copy_size <= bb_slice.len() {
                    self.cursor_background.saved_pixels[idx..idx + actual_copy_size]
                        .copy_from_slice(&bb_slice[byte_offset..byte_offset + actual_copy_size]);
                }

                idx += copy_size;
            }
        }
    }

    fn restore_cursor_background(&mut self, prev_pos: (usize, usize)) {
        let start_y: usize = prev_pos.1.saturating_sub(CURSOR_ROW_OFFSET);

        let mut idx: usize = 0; // Index for the saved pixels
        for (i, (start_x, end_x)) in CURSOR_ROWS.iter().enumerate() {
            let cursor_y = start_y as isize + i as isize;

            let count = (end_x - start_x + 1)
                .min(self.width() as isize - start_x - prev_pos.0 as isize)
                .max(0) as usize;

            let start_x = prev_pos.0 as isize + start_x;

            let bytes_per_pixel = self.info.bytes_per_pixel;

            if cursor_y < 0 || cursor_y >= self.height() as isize {
                idx += count * bytes_per_pixel; // Skip the data but advance index
                continue; // Skip out-of-bounds rows
            }
            if start_x < 0 || start_x + count as isize > self.width() as isize {
                idx += count * bytes_per_pixel; // Skip the data but advance index
                continue;
            }

            let size = count * bytes_per_pixel;
            if idx + size <= self.cursor_background.saved_pixels.len() {
                // Calculate the pixel offset
                let pixel_offset = cursor_y as usize * self.info.stride + start_x as usize;
                let byte_offset = pixel_offset * self.info.bytes_per_pixel;

                let bb_slice = self.backbuffer.buffer_mut();
                if byte_offset + size <= bb_slice.len() {
                    bb_slice[byte_offset..byte_offset + size]
                        .copy_from_slice(&self.cursor_background.saved_pixels[idx..idx + size]);
                }

                idx += size;
            }
        }
    }

    pub fn draw_line(&mut self, start: (usize, usize), end: (usize, usize), color: Color) {
        let dx = end.0 as isize - start.0 as isize;
        let dy = end.1 as isize - start.1 as isize;
        let steps = dx.abs().max(dy.abs()) as usize;

        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let x = (start.0 as f32 + t * dx as f32) as usize;
            let y = (start.1 as f32 + t * dy as f32) as usize;
            self.write_pixel(x, y, color);
        }
    }

    /// Writes a horizontal line of pixels at once.
    fn write_pixel_row(&mut self, x1: usize, x2: usize, y: usize, data: &Color) {
        let x2 = x2.min(self.width() - 1); // Ensure x2 is within bounds
        if y >= self.height() || x1 >= self.width() {
            return;
        }

        let max_width = (x2 - x1 + 1).min(self.width() - x1);
        let bytes_per_pixel = self.info.bytes_per_pixel;
        let start_offset = (y * self.info.stride + x1) * bytes_per_pixel;

        // Convert the color to the appropriate pixel format
        let color: [u8; 4] = match self.info.pixel_format {
            PixelFormat::Rgb => [data.r, data.g, data.b, 0],
            PixelFormat::Bgr => [data.b, data.g, data.r, 0],
            PixelFormat::U8 => [if data.to_u8() > 200 { 0xf } else { 0 }, 0, 0, 0],
            _ => {
                panic!(
                    "pixel format {:?} not supported for writing",
                    self.info.pixel_format
                );
            }
        };

        // Convert the color slice to the correct length
        let data: Vec<u8> = color[..bytes_per_pixel]
            .iter()
            .cloned()
            .cycle()
            .take(max_width * bytes_per_pixel)
            .collect();

        let bb_slice = self.backbuffer.buffer_mut();
        if start_offset + max_width * bytes_per_pixel <= bb_slice.len() {
            bb_slice[start_offset..start_offset + max_width * bytes_per_pixel]
                .copy_from_slice(&data);
            return;
        }
    }

    pub fn draw_rect(
        &mut self,
        top_left: (usize, usize),
        bottom_right: (usize, usize),
        color: Color,
    ) {
        let width = bottom_right.0 - top_left.0 + 1;
        let height = bottom_right.1 - top_left.1 + 1;
        self.dirty_tracker
            .mark_dirty(top_left.0, top_left.1, width, height);

        for y in top_left.1..=bottom_right.1 {
            self.write_pixel_row(top_left.0, bottom_right.0, y, &color);
        }
    }

    pub fn draw_rect_outline(
        &mut self,
        top_left: (usize, usize),
        bottom_right: (usize, usize),
        color: Color,
    ) {
        let width = bottom_right.0 - top_left.0 + 1;
        let height = bottom_right.1 - top_left.1 + 1;
        self.dirty_tracker
            .mark_dirty(top_left.0, top_left.1, width, height);

        self.write_pixel_row(top_left.0, bottom_right.0, top_left.1, &color);
        self.write_pixel_row(top_left.0, bottom_right.0, bottom_right.1, &color);

        for y in top_left.1..=bottom_right.1 {
            self.write_pixel(top_left.0, y, color);
            self.write_pixel(bottom_right.0, y, color);
        }
    }

    pub fn draw_raw_text(
        &mut self,
        text: &str,
        x: usize,
        y: usize,
        color: Color,
        bg_color: Color,
        font_weight: FontWeight,
        font_size: RasterHeight,
    ) {
        let mut x_offset = x;
        let mut y_offset = y;
        for c in text.chars() {
            match c {
                '\n' => {
                    x_offset = x; // Reset x offset for new line
                    y_offset += font_size.val() + LINE_SPACING;

                    continue;
                }
                '\r' => {
                    x_offset = x; // Reset x offset for carriage return

                    continue;
                }
                _ => {}
            }

            let rendered_char = get_char_raster(c, font_weight, font_size);

            self.write_rendered_char_at_pos(x_offset, y_offset, &rendered_char, color, bg_color);
            x_offset += LETTER_SPACING + rendered_char.width(); // Move to the next character position
        }
    }

    /// Present the backbuffer to the screen (swap buffers)
    pub fn present_backbuffer(&mut self) {
        let bb_data = self.backbuffer.buffer();
        let copy_size = bb_data.len().min(self.framebuffer.len());
        self.framebuffer[..copy_size].copy_from_slice(&bb_data[..copy_size]);
    }

    /// Present only dirty regions of the backbuffer to the screen (optimized)
    pub fn present_backbuffer_dirty(&mut self) -> usize {
        let dirty_regions = self.dirty_tracker.get_dirty_regions();
        let regions_count = dirty_regions.len();

        if regions_count == 0 {
            return 0; // Nothing to update
        }

        let bb_data = self.backbuffer.buffer();
        let bytes_per_pixel = self.info.bytes_per_pixel;
        let stride = self.info.stride;

        // Copy only dirty regions
        for (x, y) in dirty_regions {
            let x = x as usize * TILE_SIZE;
            let y = y as usize * TILE_SIZE;
            let width = TILE_SIZE.min(self.info.width - x);
            let height = TILE_SIZE.min(self.info.height - y);

            for row in y..(y + height) {
                if row >= self.info.height {
                    break;
                }

                let fb_start = (row * stride + x) * bytes_per_pixel;
                let fb_end = fb_start + (width * bytes_per_pixel);

                if fb_end <= self.framebuffer.len() && fb_end <= bb_data.len() {
                    self.framebuffer[fb_start..fb_end].copy_from_slice(&bb_data[fb_start..fb_end]);
                }
            }
        }

        regions_count
    }

    /// Clear the backbuffer
    pub fn clear_backbuffer(&mut self) {
        self.backbuffer.clear();
    }

    /// Copy entire framebuffer contents to backbuffer
    pub fn copy_to_backbuffer(&mut self) {
        self.backbuffer.copy_from(self.framebuffer);
    }

    /// Copy entire backbuffer contents to framebuffer
    pub fn copy_from_backbuffer(&mut self) {
        let bb_data = self.backbuffer.buffer();
        let copy_size = bb_data.len().min(self.framebuffer.len());
        self.framebuffer[..copy_size].copy_from_slice(&bb_data[..copy_size]);
    }

    /// Get buffer info
    pub fn get_info(&self) -> FrameBufferInfo {
        self.info
    }

    /// Mark a region as dirty for optimized rendering
    pub fn mark_dirty_region(&mut self, x: usize, y: usize, width: usize, height: usize) {
        self.dirty_tracker.mark_dirty(x, y, width, height);
    }

    /// Mark the entire screen as dirty
    pub fn mark_all_dirty(&mut self) {
        self.dirty_tracker.mark_all_dirty();
    }

    /// Check if there are any dirty regions
    pub fn has_dirty_regions(&self) -> bool {
        self.dirty_tracker.has_dirty_tiles()
    }

    /// Get the number of dirty tiles (for debugging/profiling)
    pub fn get_dirty_tile_count(&self) -> usize {
        self.dirty_tracker
            .dirty_tiles
            .iter()
            .filter(|&&dirty| dirty)
            .count()
    }
}

unsafe impl Send for FrameBufferWriter {}
unsafe impl Sync for FrameBufferWriter {}

unsafe impl Send for BackBuffer {}
unsafe impl Sync for BackBuffer {}

impl fmt::Write for FrameBufferWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.write_char(c);
        }
        Ok(())
    }
}

pub fn init(
    frame: &'static mut FrameBuffer,
    mapper: &mut impl Mapper<Size2MiB>,
    frame_allocator: &mut (impl FrameAllocator<Size2MiB> + FrameAllocator<Size4KiB>),
) {
    SCREEN_SIZE.init_once(|| {
        let info = frame.info();
        (info.width as u16, info.height as u16)
    });

    FRAMEBUFFER.init_once(|| {
        let info = frame.info();
        let buffer = frame.buffer_mut();

        spinning_top::Spinlock::new(FrameBufferWriter::new(
            buffer,
            info,
            frame_allocator,
            mapper,
        ))
    });
}

/// Present the backbuffer to the screen
pub fn present() {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let mut fb = framebuffer.lock();
        fb.present_backbuffer();
    }
}

/// Present only dirty regions of the backbuffer to the screen (optimized)
pub fn present_dirty() -> usize {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let mut fb = framebuffer.lock();
        fb.present_backbuffer_dirty()
    } else {
        0
    }
}

/// Mark a region as dirty for optimized rendering
pub fn mark_dirty_region(x: usize, y: usize, width: usize, height: usize) {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let mut fb = framebuffer.lock();
        fb.mark_dirty_region(x, y, width, height);
    }
}

/// Check if there are dirty regions that need updating
pub fn has_dirty_regions() -> bool {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let fb = framebuffer.lock();
        fb.has_dirty_regions()
    } else {
        false
    }
}

/// Clear the backbuffer
pub fn clear_backbuffer() {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let mut fb = framebuffer.lock();
        fb.clear_backbuffer();
    }
}

pub fn set_buffer(buffer: &[u8]) {
    if let Some(fb) = FRAMEBUFFER.get() {
        fb.lock().framebuffer.copy_from_slice(buffer);
    } else {
        panic!("FrameBuffer not initialized");
    }
}

/// Read a pixel from the active buffer (backbuffer if enabled, framebuffer otherwise)
pub fn read_pixel(x: usize, y: usize) -> Option<Color> {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let fb = framebuffer.lock();
        Some(fb.read_pixel(x, y))
    } else {
        None
    }
}

/// Write a pixel to the active buffer (backbuffer if enabled, framebuffer otherwise)
pub fn write_pixel(x: usize, y: usize, color: Color) {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let mut fb = framebuffer.lock();
        fb.write_pixel(x, y, color);
    }
}

/// Read a row of pixels from the active buffer
pub fn read_pixel_row(x: usize, y: usize, count: usize) -> Vec<u8> {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let fb = framebuffer.lock();
        fb.read_raw_pixel_row(x, y, count)
    } else {
        Vec::new()
    }
}

/// Write a row of pixels to the active buffer
pub fn write_pixel_row(x: usize, y: usize, data: &[u8]) {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let mut fb = framebuffer.lock();
        fb.write_raw_pixel_row(x, y, data);
    }
}

/// Fill the active buffer with a single brightness value
pub fn fill_buffer(brightness: u8) {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let mut fb = framebuffer.lock();
        fb.fill(brightness);
    }
}

/// Clear the active buffer
pub fn clear_buffer() {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let mut fb = framebuffer.lock();
        fb.clear();
    }
}

/// Get screen dimensions
pub fn get_screen_size() -> Option<(usize, usize)> {
    if let Some(framebuffer) = FRAMEBUFFER.get() {
        let fb = framebuffer.lock();
        Some(fb.size())
    } else {
        None
    }
}
