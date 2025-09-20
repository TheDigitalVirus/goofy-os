use alloc::{
    format,
    string::{String, ToString},
};
use pc_keyboard::KeyCode;

use crate::{
    desktop::application::Application,
    framebuffer::Color,
    surface::{Shape, Surface},
};
use noto_sans_mono_bitmap::{FontWeight, RasterHeight};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Player {
    X,
    O,
}

impl Player {
    pub fn other(self) -> Self {
        match self {
            Player::X => Player::O,
            Player::O => Player::X,
        }
    }

    pub fn to_string(self) -> &'static str {
        match self {
            Player::X => "X",
            Player::O => "O",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cell {
    Empty,
    Occupied(Player),
}

#[derive(Clone, Debug, PartialEq)]
pub enum GameMode {
    TwoPlayer,
    VsBot,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GameState {
    Playing,
    Won(Player),
    Draw,
}

pub struct TicTacToe {
    board: [[Cell; 3]; 3],
    current_player: Player,
    game_state: GameState,
    game_mode: GameMode,
    status_message: String,

    // UI element indices
    status_text_idx: Option<usize>,
    cell_indices: [[Option<usize>; 3]; 3], // Background shapes for cells
    cell_text_indices: [[Option<usize>; 3]; 3], // Text shapes for X/O

    // Button indices
    new_game_btn_idx: Option<usize>,
    two_player_btn_idx: Option<usize>,
    vs_bot_btn_idx: Option<usize>,
}

impl TicTacToe {
    pub fn new(_args: Option<String>) -> Self {
        Self {
            board: [[Cell::Empty; 3]; 3],
            current_player: Player::X,
            game_state: GameState::Playing,
            game_mode: GameMode::TwoPlayer,
            status_message: "Player X's turn".to_string(),
            status_text_idx: None,
            cell_indices: [[None; 3]; 3],
            cell_text_indices: [[None; 3]; 3],
            new_game_btn_idx: None,
            two_player_btn_idx: None,
            vs_bot_btn_idx: None,
        }
    }

    fn setup_ui(&mut self, surface: &mut Surface) {
        surface.clear_all_shapes();

        let width = surface.width;

        // Title
        surface.add_shape(Shape::Text {
            x: width / 2 - 50,
            y: 20,
            content: "Tic-Tac-Toe".to_string(),
            color: Color::BLACK,
            background_color: surface.background_color,
            font_size: RasterHeight::Size24,
            font_weight: FontWeight::Bold,
            hide: false,
        });

        // Game mode buttons
        let mode_color_2p = if self.game_mode == GameMode::TwoPlayer {
            Color::new(200, 255, 200)
        } else {
            Color::new(220, 220, 220)
        };

        let mode_color_bot = if self.game_mode == GameMode::VsBot {
            Color::new(200, 255, 200)
        } else {
            Color::new(220, 220, 220)
        };

        self.two_player_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: 50,
            y: 50,
            width: 80,
            height: 30,
            color: mode_color_2p,
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: 50,
            y: 50,
            width: 80,
            height: 30,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: 65,
            y: 60,
            content: "2 Player".to_string(),
            color: Color::BLACK,
            background_color: mode_color_2p,
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        self.vs_bot_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: 140,
            y: 50,
            width: 80,
            height: 30,
            color: mode_color_bot,
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: 140,
            y: 50,
            width: 80,
            height: 30,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: 155,
            y: 60,
            content: "vs Bot".to_string(),
            color: Color::BLACK,
            background_color: mode_color_bot,
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });

        // Game board - 3x3 grid
        let board_start_x = 80;
        let board_start_y = 100;
        let cell_size = 60;
        let cell_padding = 5;

        for row in 0..3 {
            for col in 0..3 {
                let x = board_start_x + col * (cell_size + cell_padding);
                let y = board_start_y + row * (cell_size + cell_padding);

                // Cell background
                self.cell_indices[row][col] = Some(surface.add_shape(Shape::Rectangle {
                    x,
                    y,
                    width: cell_size,
                    height: cell_size,
                    color: Color::WHITE,
                    filled: true,
                    hide: false,
                }));

                // Cell border
                surface.add_shape(Shape::Rectangle {
                    x,
                    y,
                    width: cell_size,
                    height: cell_size,
                    color: Color::BLACK,
                    filled: false,
                    hide: false,
                });

                // Cell text (X or O)
                let text_content = match self.board[row][col] {
                    Cell::Empty => " ".to_string(),
                    Cell::Occupied(player) => player.to_string().to_string(),
                };

                self.cell_text_indices[row][col] = Some(surface.add_shape(Shape::Text {
                    x: x + cell_size / 2 - 8,
                    y: y + cell_size / 2 - 8,
                    content: text_content,
                    color: Color::BLACK,
                    background_color: Color::WHITE,
                    font_size: RasterHeight::Size24,
                    font_weight: FontWeight::Bold,
                    hide: false,
                }));
            }
        }

        // Status message
        self.status_text_idx = Some(surface.add_shape(Shape::Text {
            x: 50,
            y: 320,
            content: self.status_message.clone(),
            color: Color::BLACK,
            background_color: surface.background_color,
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        }));

        // New Game button
        self.new_game_btn_idx = Some(surface.add_shape(Shape::Rectangle {
            x: 80,
            y: 350,
            width: 100,
            height: 30,
            color: Color::new(200, 200, 255),
            filled: true,
            hide: false,
        }));

        surface.add_shape(Shape::Rectangle {
            x: 80,
            y: 350,
            width: 100,
            height: 30,
            color: Color::BLACK,
            filled: false,
            hide: false,
        });

        surface.add_shape(Shape::Text {
            x: 110,
            y: 360,
            content: "New Game".to_string(),
            color: Color::BLACK,
            background_color: Color::new(200, 200, 255),
            font_size: RasterHeight::Size16,
            font_weight: FontWeight::Regular,
            hide: false,
        });
    }

    fn update_status_message(&mut self, surface: &mut Surface, message: String) {
        self.status_message = message;
        if let Some(idx) = self.status_text_idx {
            surface.update_text_content(idx, self.status_message.clone(), None);
        }
    }

    fn update_board_display(&mut self, surface: &mut Surface) {
        for row in 0..3 {
            for col in 0..3 {
                if let Some(text_idx) = self.cell_text_indices[row][col] {
                    let text_content = match self.board[row][col] {
                        Cell::Empty => " ".to_string(),
                        Cell::Occupied(player) => player.to_string().to_string(),
                    };
                    surface.update_text_content(text_idx, text_content, None);
                }
            }
        }
    }

    fn make_move(&mut self, row: usize, col: usize, surface: &mut Surface) -> bool {
        if self.game_state != GameState::Playing || self.board[row][col] != Cell::Empty {
            return false;
        }

        // Make the move
        self.board[row][col] = Cell::Occupied(self.current_player);

        // Update display
        self.update_board_display(surface);

        // Check for win or draw
        self.check_game_state();

        // Update status message
        match &self.game_state {
            GameState::Won(player) => {
                self.update_status_message(surface, format!("Player {} wins!", player.to_string()));
            }
            GameState::Draw => {
                self.update_status_message(surface, "It's a draw!".to_string());
            }
            GameState::Playing => {
                self.current_player = self.current_player.other();

                // If it's bot mode and now it's O's turn, make bot move
                if self.game_mode == GameMode::VsBot && self.current_player == Player::O {
                    self.update_status_message(surface, "Bot is thinking...".to_string());
                    // Bot move will be made in the next frame
                } else {
                    let player_name =
                        if self.game_mode == GameMode::VsBot && self.current_player == Player::X {
                            "Your"
                        } else {
                            &format!("Player {}", self.current_player.to_string())
                        };
                    self.update_status_message(surface, format!("{} turn", player_name));
                }
            }
        }

        true
    }

    fn make_bot_move(&mut self, surface: &mut Surface) {
        if self.game_state != GameState::Playing || self.current_player != Player::O {
            return;
        }

        // Simple bot strategy:
        // 1. Try to win
        // 2. Try to block player from winning
        // 3. Take center if available
        // 4. Take corners
        // 5. Take sides

        if let Some((row, col)) = self.find_winning_move(Player::O) {
            self.make_move(row, col, surface);
            return;
        }

        if let Some((row, col)) = self.find_winning_move(Player::X) {
            self.make_move(row, col, surface);
            return;
        }

        // Take center
        if self.board[1][1] == Cell::Empty {
            self.make_move(1, 1, surface);
            return;
        }

        // Take corners
        let corners = [(0, 0), (0, 2), (2, 0), (2, 2)];
        for (row, col) in corners.iter() {
            if self.board[*row][*col] == Cell::Empty {
                self.make_move(*row, *col, surface);
                return;
            }
        }

        // Take sides
        let sides = [(0, 1), (1, 0), (1, 2), (2, 1)];
        for (row, col) in sides.iter() {
            if self.board[*row][*col] == Cell::Empty {
                self.make_move(*row, *col, surface);
                return;
            }
        }
    }

    fn find_winning_move(&self, player: Player) -> Option<(usize, usize)> {
        for row in 0..3 {
            for col in 0..3 {
                if self.board[row][col] == Cell::Empty {
                    // Try this move
                    let mut test_board = self.board;
                    test_board[row][col] = Cell::Occupied(player);

                    if self.check_win_on_board(&test_board, player) {
                        return Some((row, col));
                    }
                }
            }
        }
        None
    }

    fn check_game_state(&mut self) {
        // Check for wins
        if self.check_win(Player::X) {
            self.game_state = GameState::Won(Player::X);
        } else if self.check_win(Player::O) {
            self.game_state = GameState::Won(Player::O);
        } else if self.is_board_full() {
            self.game_state = GameState::Draw;
        }
    }

    fn check_win(&self, player: Player) -> bool {
        self.check_win_on_board(&self.board, player)
    }

    fn check_win_on_board(&self, board: &[[Cell; 3]; 3], player: Player) -> bool {
        let target = Cell::Occupied(player);

        // Check rows
        for row in 0..3 {
            if board[row][0] == target && board[row][1] == target && board[row][2] == target {
                return true;
            }
        }

        // Check columns
        for col in 0..3 {
            if board[0][col] == target && board[1][col] == target && board[2][col] == target {
                return true;
            }
        }

        // Check diagonals
        if board[0][0] == target && board[1][1] == target && board[2][2] == target {
            return true;
        }
        if board[0][2] == target && board[1][1] == target && board[2][0] == target {
            return true;
        }

        false
    }

    fn is_board_full(&self) -> bool {
        for row in 0..3 {
            for col in 0..3 {
                if self.board[row][col] == Cell::Empty {
                    return false;
                }
            }
        }
        true
    }

    fn new_game(&mut self, surface: &mut Surface) {
        self.board = [[Cell::Empty; 3]; 3];
        self.current_player = Player::X;
        self.game_state = GameState::Playing;

        let message = if self.game_mode == GameMode::VsBot {
            "Your turn (X)"
        } else {
            "Player X's turn"
        };

        self.update_status_message(surface, message.to_string());
        self.update_board_display(surface);
    }

    fn set_game_mode(&mut self, mode: GameMode, surface: &mut Surface) {
        self.game_mode = mode;
        self.new_game(surface);
        self.setup_ui(surface); // Refresh UI to update button colors
    }

    fn handle_cell_click(&mut self, x: usize, y: usize, surface: &mut Surface) -> bool {
        if self.game_state != GameState::Playing {
            return false;
        }

        // If it's bot mode and it's the bot's turn, ignore clicks
        if self.game_mode == GameMode::VsBot && self.current_player == Player::O {
            return false;
        }

        let board_start_x = 80;
        let board_start_y = 100;
        let cell_size = 60;
        let cell_padding = 5;

        for row in 0..3 {
            for col in 0..3 {
                let cell_x = board_start_x + col * (cell_size + cell_padding);
                let cell_y = board_start_y + row * (cell_size + cell_padding);

                if x >= cell_x && x < cell_x + cell_size && y >= cell_y && y < cell_y + cell_size {
                    return self.make_move(row, col, surface);
                }
            }
        }

        false
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
}

impl Application for TicTacToe {
    fn init(&mut self, surface: &mut Surface) {
        self.setup_ui(surface);
    }

    fn handle_char_input(&mut self, _c: char, _ctrl_pressed: bool, _surface: &mut Surface) {
        // TicTacToe doesn't need character input
    }

    fn handle_key_input(&mut self, _key: KeyCode, _surface: &mut Surface) {
        // TicTacToe doesn't need keyboard input
    }

    fn handle_mouse_click(&mut self, x: usize, y: usize, surface: &mut Surface) {
        // Handle game mode buttons
        if self.is_button_clicked(x, y, 50, 50, 80, 30) {
            self.set_game_mode(GameMode::TwoPlayer, surface);
            return;
        }

        if self.is_button_clicked(x, y, 140, 50, 80, 30) {
            self.set_game_mode(GameMode::VsBot, surface);
            return;
        }

        // Handle new game button
        if self.is_button_clicked(x, y, 80, 350, 100, 30) {
            self.new_game(surface);
            return;
        }

        // Handle cell clicks
        self.handle_cell_click(x, y, surface);

        // If we're in bot mode and it's now the bot's turn, make bot move
        if self.game_mode == GameMode::VsBot
            && self.current_player == Player::O
            && self.game_state == GameState::Playing
        {
            self.make_bot_move(surface);
        }
    }

    fn render(&mut self, _surface: &mut Surface) {
        // Rendering is handled by the surface system
    }

    fn get_title(&self) -> Option<String> {
        Some("Tic-Tac-Toe".to_string())
    }
}
