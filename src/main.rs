use ggez::conf::{WindowMode, WindowSetup};
use ggez::event::{self, EventHandler, MouseButton};
use ggez::glam::*;
// Removed Mesh from graphics import line
use ggez::graphics::{self, Color, DrawMode, DrawParam, Rect, Text, TextFragment};
use ggez::{Context, ContextBuilder, GameError, GameResult};
use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
// Removed Duration from time import line
use std::time::Instant;

// --- Constants ---
const BOARD_SIZE: usize = 15;
const CELL_SIZE: f32 = 40.0;
const MARGIN: f32 = 50.0;
const BOARD_DIM: f32 = CELL_SIZE * (BOARD_SIZE as f32 - 1.0);
const WINDOW_WIDTH: f32 = BOARD_DIM + 2.0 * MARGIN;
const WINDOW_HEIGHT: f32 = BOARD_DIM + 2.0 * MARGIN + 50.0; // Extra space for text
const PIECE_RADIUS: f32 = CELL_SIZE * 0.45;
const DOT_RADIUS: f32 = 4.0;
const HIGHLIGHT_SIZE: f32 = CELL_SIZE * 0.2;

// --- AI Configuration ---
const AI_DEPTH: u32 = 3; // Keep depth 3 for reasonable speed with complex eval

// --- Evaluation Scores (Needs careful tuning!) ---
const SCORE_FIVE: i32 = 100_000; // 连五 (Winning state)

// Compound Threats (Highest priority after Five)
const SCORE_FOUR_THREE: i32 = 50_000; // 冲四活三 或 活四活三 (Near win)
const SCORE_DOUBLE_LIVE_THREE: i32 = 5_000; // 双活三 (Very strong attack)

// Single Pattern Scores
const SCORE_LIVE_FOUR: i32 = 8_000; // 活四
const SCORE_DOUBLE_DEAD_FOUR: i32 = 4_000; // 双冲四 (Can force a win or strong defense)
const SCORE_DEAD_FOUR: i32 = 1_000; // 冲四 / 死四
const SCORE_LIVE_THREE: i32 = 900; // 活三
const SCORE_JUMP_LIVE_THREE: i32 = 850; // 跳活三 (Slightly less obvious L3)
const SCORE_SLEEPY_THREE: i32 = 150; // 眠三 (One side blocked, potential)
                                     // const SCORE_DEAD_THREE: i32 = 5;       // 死三 (Low value) - Often omitted or very low score
const SCORE_LIVE_TWO: i32 = 100; // 活二
                                 //const SCORE_JUMP_LIVE_TWO: i32 = 90; // 跳活二
const SCORE_SLEEPY_TWO: i32 = 10; // 眠二
                                  // const SCORE_DEAD_TWO: i32 = 1;         // 死二 (Very low value) - Often omitted
                                  // const SCORE_LIVE_ONE: i32 = 1;         // 活一 (Low value)
                                  // const SCORE_DEAD_ONE: i32 = 0;         // 死一 (No value)

// --- Enums and Structs ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Player {
    Black,
    White,
}

impl Player {
    fn opponent(&self) -> Player {
        match self {
            Player::Black => Player::White,
            Player::White => Player::Black,
        }
    }

    fn color(&self) -> Color {
        match self {
            Player::Black => Color::BLACK,
            Player::White => Color::WHITE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GamePhase {
    ChooseSide,
    Playing,
    GameOver,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GameResultState {
    Winner(Player),
    Draw,
}

type Board = [[Option<Player>; BOARD_SIZE]; BOARD_SIZE];

// Represents a potential threat formed by placing a piece at an empty cell
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ThreatType {
    // Five, // Five is usually a winning state, handled separately or highest score
    LiveFour,
    DeadFour,
    LiveThree,
    SleepyThree, // Includes jump sleepy threes for simplicity here
    LiveTwo,
    SleepyTwo, // Includes jump sleepy twos
}

// Map from empty cell coordinate (x, y) to list of threats formed by playing there
type ThreatMap = HashMap<(usize, usize), Vec<(ThreatType, Player)>>;

struct GameState {
    board: Board,
    current_player: Player,
    human_player: Player,
    phase: GamePhase,
    game_result: Option<GameResultState>,
    last_move: Option<(usize, usize)>,
    move_history: Vec<(usize, usize, Player)>,
    ai_thinking: bool,
    ai_move_receiver: Option<mpsc::Receiver<(usize, usize)>>,
    ai_thread_handle: Option<thread::JoinHandle<()>>,
    status_text: String,
}

impl GameState {
    fn new(_ctx: &mut Context) -> GameResult<GameState> {
        Ok(GameState {
            board: [[None; BOARD_SIZE]; BOARD_SIZE],
            current_player: Player::Black,
            human_player: Player::Black,
            phase: GamePhase::ChooseSide,
            game_result: None,
            last_move: None,
            move_history: Vec::new(),
            ai_thinking: false,
            ai_move_receiver: None,
            ai_thread_handle: None,
            status_text: "Choose your side: Press 'B' for Black, 'W' for White".to_string(),
        })
    }

    fn screen_to_grid(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        if x < MARGIN - CELL_SIZE / 2.0
            || x > MARGIN + BOARD_DIM + CELL_SIZE / 2.0
            || y < MARGIN - CELL_SIZE / 2.0
            || y > MARGIN + BOARD_DIM + CELL_SIZE / 2.0
        {
            return None;
        }
        let grid_x = ((x - MARGIN + CELL_SIZE / 2.0) / CELL_SIZE).floor() as usize;
        let grid_y = ((y - MARGIN + CELL_SIZE / 2.0) / CELL_SIZE).floor() as usize;
        if grid_x < BOARD_SIZE && grid_y < BOARD_SIZE {
            Some((grid_x, grid_y))
        } else {
            None
        }
    }

    fn grid_to_screen(&self, x: usize, y: usize) -> Vec2 {
        Vec2::new(MARGIN + x as f32 * CELL_SIZE, MARGIN + y as f32 * CELL_SIZE)
    }

    fn make_move(&mut self, x: usize, y: usize, player: Player) -> Result<(), &'static str> {
        if self.game_result.is_some() {
            return Err("Game is already over.");
        }
        if x >= BOARD_SIZE || y >= BOARD_SIZE {
            return Err("Move out of bounds.");
        }
        if self.board[y][x].is_some() {
            return Err("Cell is already occupied.");
        }

        self.board[y][x] = Some(player);
        self.last_move = Some((x, y));
        self.move_history.push((x, y, player));

        if self.check_win(x, y, player) {
            self.phase = GamePhase::GameOver;
            self.game_result = Some(GameResultState::Winner(player));
            self.status_text = format!(
                "{} wins! Press 'R' to restart.",
                if player == Player::Black {
                    "Black"
                } else {
                    "White"
                }
            );
        } else if self.check_draw() {
            self.phase = GamePhase::GameOver;
            self.game_result = Some(GameResultState::Draw);
            self.status_text = "It's a draw! Press 'R' to restart.".to_string();
        } else {
            self.current_player = player.opponent();
            self.update_status_text();
        }
        Ok(())
    }

    fn check_win(&self, x: usize, y: usize, player: Player) -> bool {
        let directions = [(0, 1), (1, 0), (1, 1), (1, -1)]; // H, V, Diag\, Diag/
        for (dx, dy) in directions {
            let mut count = 1;
            // Forward
            for i in 1..5 {
                let nx = x as isize + dx * i;
                let ny = y as isize + dy * i;
                if nx >= 0
                    && nx < BOARD_SIZE as isize
                    && ny >= 0
                    && ny < BOARD_SIZE as isize
                    && self.board[ny as usize][nx as usize] == Some(player)
                {
                    count += 1;
                } else {
                    break;
                }
            }
            // Backward
            for i in 1..5 {
                let nx = x as isize - dx * i;
                let ny = y as isize - dy * i;
                if nx >= 0
                    && nx < BOARD_SIZE as isize
                    && ny >= 0
                    && ny < BOARD_SIZE as isize
                    && self.board[ny as usize][nx as usize] == Some(player)
                {
                    count += 1;
                } else {
                    break;
                }
            }
            if count >= 5 {
                return true;
            }
        }
        false
    }

    fn check_draw(&self) -> bool {
        self.move_history.len() == BOARD_SIZE * BOARD_SIZE
    }

    fn undo_move(&mut self) {
        if self.phase != GamePhase::Playing || self.ai_thinking {
            println!("Cannot undo now.");
            return;
        }
        let num_moves_to_undo = if self.move_history.len() >= 2 {
            2
        } else {
            self.move_history.len()
        };
        if num_moves_to_undo == 0 {
            println!("No moves to undo.");
            return;
        }
        for _ in 0..num_moves_to_undo {
            if let Some((x, y, player)) = self.move_history.pop() {
                self.board[y][x] = None;
                self.current_player = player;
            }
        }
        self.last_move = self.move_history.last().map(|(x, y, _)| (*x, *y));
        self.game_result = None;
        self.phase = GamePhase::Playing;
        self.update_status_text();
        println!("Undo successful. {} moves undone.", num_moves_to_undo);
    }

    fn reset_game(&mut self) {
        self.board = [[None; BOARD_SIZE]; BOARD_SIZE];
        self.current_player = Player::Black;
        self.phase = GamePhase::ChooseSide;
        self.game_result = None;
        self.last_move = None;
        self.move_history.clear();
        self.ai_thinking = false;
        // Fix warning: unused variable handle
        if let Some(_handle) = self.ai_thread_handle.take() {
            // Let thread detach, don't join
        }
        self.ai_move_receiver = None;
        self.status_text = "Choose your side: Press 'B' for Black, 'W' for White".to_string();
    }

    fn update_status_text(&mut self) {
        if self.phase == GamePhase::GameOver {
            return;
        } // Set in make_move
        if self.phase == GamePhase::ChooseSide {
            self.status_text = "Choose your side: Press 'B' for Black, 'W' for White".to_string();
            return;
        }
        let current_player_name = if self.current_player == Player::Black {
            "Black"
        } else {
            "White"
        };
        let turn_info = if self.current_player == self.human_player {
            format!("Your turn ({})", current_player_name)
        } else {
            "AI is thinking...".to_string()
        };
        self.status_text = format!("{} | Press 'U' to Undo, 'R' to Restart", turn_info);
    }

    fn trigger_ai_move(&mut self) {
        if self.current_player == self.human_player
            || self.phase != GamePhase::Playing
            || self.ai_thinking
        {
            return;
        }
        self.ai_thinking = true;
        self.update_status_text();

        let (sender, receiver) = mpsc::channel();
        self.ai_move_receiver = Some(receiver);
        let current_board = self.board;
        let ai_player = self.current_player;

        self.ai_thread_handle = Some(thread::spawn(move || {
            let start_time = Instant::now();
            println!(
                "AI ({:?}) starting minimax (Depth {})...",
                ai_player, AI_DEPTH
            );
            let best_move = find_best_move(current_board, ai_player, AI_DEPTH);
            let duration = start_time.elapsed();
            println!(
                "AI ({:?}) finished in {:?}. Best move: {:?}",
                ai_player, duration, best_move
            );
            if let Some(mv) = best_move {
                let _ = sender.send(mv);
            } else {
                println!("AI Error: No valid move found! Sending fallback.");
                let fallback_move = find_first_empty_cell(current_board);
                if let Some(mv) = fallback_move {
                    let _ = sender.send(mv);
                } else {
                    println!("AI Error: Board is full, cannot send fallback.");
                }
            }
        }));
    }

    fn handle_ai_result(&mut self) {
        if let Some(receiver) = &self.ai_move_receiver {
            match receiver.try_recv() {
                Ok((x, y)) => {
                    let ai_player = self.current_player;
                    match self.make_move(x, y, ai_player) {
                        Ok(_) => println!("AI placed at ({}, {})", x, y),
                        Err(e) => {
                            eprintln!("AI Error: Invalid move ({}, {}) calculated: {}", x, y, e);
                            self.current_player = ai_player.opponent(); // Give turn back? Risky.
                        }
                    }
                    self.ai_thinking = false;
                    self.ai_move_receiver = None;
                    self.ai_thread_handle = None;
                    self.update_status_text();
                }
                Err(mpsc::TryRecvError::Empty) => { /* Still thinking */ }
                Err(mpsc::TryRecvError::Disconnected) => {
                    eprintln!("AI thread disconnected unexpectedly!");
                    self.ai_thinking = false;
                    self.ai_move_receiver = None;
                    self.ai_thread_handle = None;
                    self.status_text = "AI Error! Press 'R' to restart.".to_string();
                    self.phase = GamePhase::GameOver;
                }
            }
        }
    }
}

// --- AI Logic ---

fn find_best_move(board: Board, ai_player: Player, depth: u32) -> Option<(usize, usize)> {
    let (_, best_move_opt) = minimax(board, depth, i32::MIN + 1, i32::MAX - 1, true, ai_player);
    best_move_opt
}

fn minimax(
    mut board: Board,
    depth: u32,
    mut alpha: i32,
    mut beta: i32,
    maximizing_player: bool,
    ai_player: Player,
) -> (i32, Option<(usize, usize)>) {
    if let Some(winner) = check_immediate_win(&board) {
        let score = if winner == ai_player {
            SCORE_FIVE
        } else {
            -SCORE_FIVE
        };
        return (
            score
                + if maximizing_player {
                    -(depth as i32)
                } else {
                    depth as i32
                },
            None,
        );
    }
    if is_board_full(&board) {
        return (0, None); // Draw
    }
    if depth == 0 {
        return (evaluate_board(&board, ai_player), None);
    }

    let current_eval_player = if maximizing_player {
        ai_player
    } else {
        ai_player.opponent()
    };
    let moves = generate_moves(&board);

    if moves.is_empty() {
        return (evaluate_board(&board, ai_player), None);
    }

    // Fix E0382: Get the potential fallback move *before* iterating/moving `moves`
    let first_move_fallback = moves.first().copied();

    let mut best_move: Option<(usize, usize)> = None;

    if maximizing_player {
        let mut max_eval = i32::MIN;
        for (x, y) in moves {
            // Iterate over moves (consumes/borrows moves)
            board[y][x] = Some(current_eval_player); // Make move
            let (eval, _) = minimax(board, depth - 1, alpha, beta, false, ai_player);
            board[y][x] = None; // Undo move

            if eval > max_eval {
                max_eval = eval;
                best_move = Some((x, y));
            }
            alpha = alpha.max(eval);
            if beta <= alpha {
                break; // Beta cutoff
            }
        }
        // Use the pre-calculated fallback if no better move was found
        (max_eval, best_move.or(first_move_fallback))
    } else {
        // Minimizing player
        let mut min_eval = i32::MAX;
        for (x, y) in moves {
            // Iterate over moves
            board[y][x] = Some(current_eval_player); // Make move
            let (eval, _) = minimax(board, depth - 1, alpha, beta, true, ai_player);
            board[y][x] = None; // Undo move

            if eval < min_eval {
                min_eval = eval;
                best_move = Some((x, y));
            }
            beta = beta.min(eval);
            if beta <= alpha {
                break; // Alpha cutoff
            }
        }
        // Use the pre-calculated fallback if no better move was found
        (min_eval, best_move.or(first_move_fallback))
    }
}

// Check if board is full
fn is_board_full(board: &Board) -> bool {
    board
        .iter()
        .all(|row| row.iter().all(|cell| cell.is_some()))
}

// Check if a player has already won (used for early termination in minimax)
fn check_immediate_win(board: &Board) -> Option<Player> {
    for r in 0..BOARD_SIZE {
        for c in 0..BOARD_SIZE {
            if let Some(player) = board[r][c] {
                if c + 4 < BOARD_SIZE && (1..5).all(|i| board[r][c + i] == Some(player)) {
                    return Some(player);
                } // H
                if r + 4 < BOARD_SIZE && (1..5).all(|i| board[r + i][c] == Some(player)) {
                    return Some(player);
                } // V
                if c + 4 < BOARD_SIZE
                    && r + 4 < BOARD_SIZE
                    && (1..5).all(|i| board[r + i][c + i] == Some(player))
                {
                    return Some(player);
                } // Diag \
                if c >= 4
                    && r + 4 < BOARD_SIZE
                    && (1..5).all(|i| board[r + i][c - i] == Some(player))
                {
                    return Some(player);
                } // Diag /
            }
        }
    }
    None
}

// Generate candidate moves (Optimized: only near existing pieces)
fn generate_moves(board: &Board) -> Vec<(usize, usize)> {
    let mut moves = Vec::new();
    const SEARCH_RANGE: usize = 1;
    let mut relevant_empty_cells = std::collections::HashSet::new();
    let mut has_pieces = false;

    for r in 0..BOARD_SIZE {
        for c in 0..BOARD_SIZE {
            if board[r][c].is_some() {
                has_pieces = true;
                let min_r = r.saturating_sub(SEARCH_RANGE);
                let max_r = (r + SEARCH_RANGE).min(BOARD_SIZE - 1);
                let min_c = c.saturating_sub(SEARCH_RANGE);
                let max_c = (c + SEARCH_RANGE).min(BOARD_SIZE - 1);

                for nr in min_r..=max_r {
                    for nc in min_c..=max_c {
                        if board[nr][nc].is_none() {
                            relevant_empty_cells.insert((nc, nr));
                        }
                    }
                }
            }
        }
    }

    if !has_pieces {
        moves.push((BOARD_SIZE / 2, BOARD_SIZE / 2));
    } else {
        moves = relevant_empty_cells.into_iter().collect();
        if moves.is_empty() {
            if let Some(fallback) = find_first_empty_cell(*board) {
                moves.push(fallback);
            }
        }
    }
    moves
}

// Find the first empty cell (fallback for generate_moves)
fn find_first_empty_cell(board: Board) -> Option<(usize, usize)> {
    for r in 0..BOARD_SIZE {
        for c in 0..BOARD_SIZE {
            if board[r][c].is_none() {
                return Some((c, r));
            }
        }
    }
    None
}

// --- Advanced Evaluation Functions ---

// Evaluate the entire board state from ai_player's perspective
fn evaluate_board(board: &Board, ai_player: Player) -> i32 {
    let opponent_player = ai_player.opponent();
    let mut ai_base_score = 0;
    let mut opp_base_score = 0;
    let mut threat_map: ThreatMap = HashMap::new();

    // Fix E0596: Declare process_line as mutable because it captures mutably
    let mut process_line = |line: &[Option<Player>], coords: &[(usize, usize)]| {
        // Fix: Ensure line is long enough for a window size of 6
        // if line.len() < 5 { return; } // Incorrect check
        if line.len() < 6 {
            return;
        } // Correct check for window_size = 6
        let (line_ai_score, line_opp_score, line_threats) =
            evaluate_line_detailed(line, ai_player, opponent_player);
        ai_base_score += line_ai_score;
        opp_base_score += line_opp_score;

        // Map line threats back to global coordinates (x, y) and merge
        for (index_in_line, threats_at_index) in line_threats {
            if index_in_line < coords.len() {
                let (r, c) = coords[index_in_line]; // Get (row, col) from the precomputed coords
                let global_coord = (c, r); // Use (x, y) for map key
                threat_map
                    .entry(global_coord)
                    .or_default()
                    .extend(threats_at_index);
            }
        }
    };

    // Rows
    for r in 0..BOARD_SIZE {
        let line: Vec<_> = board[r].to_vec();
        let coords: Vec<_> = (0..BOARD_SIZE).map(|c| (r, c)).collect();
        process_line(&line, &coords);
    }
    // Columns
    for c in 0..BOARD_SIZE {
        let line: Vec<_> = (0..BOARD_SIZE).map(|r| board[r][c]).collect();
        let coords: Vec<_> = (0..BOARD_SIZE).map(|r| (r, c)).collect();
        process_line(&line, &coords);
    }
    // Diagonals (\) top-left to bottom-right
    // Ensure minimum length of 6 for diagonal lines processing
    for k in -(BOARD_SIZE as isize - 6)..=(BOARD_SIZE as isize - 6) {
        // Adjusted range for min length 6
        let mut line = Vec::new();
        let mut coords = Vec::new();
        for r in 0..BOARD_SIZE {
            let c = r as isize + k;
            if c >= 0 && c < BOARD_SIZE as isize {
                line.push(board[r][c as usize]);
                coords.push((r, c as usize));
            }
        }
        process_line(&line, &coords); // process_line now has the correct check internally
    }
    // Diagonals (/) top-right to bottom-left
    // Ensure minimum length of 6 for diagonal lines processing
    for k in 5..=(2 * (BOARD_SIZE - 1) - 5) {
        // Adjusted range for min length 6
        let mut line = Vec::new();
        let mut coords = Vec::new();
        for r in 0..BOARD_SIZE {
            let c = k as isize - r as isize;
            if c >= 0 && c < BOARD_SIZE as isize {
                line.push(board[r][c as usize]);
                coords.push((r, c as usize));
            }
        }
        process_line(&line, &coords); // process_line now has the correct check internally
    }

    // --- Evaluate Compound Threats from threat_map ---
    let mut ai_compound_score = 0;
    let mut opp_compound_score = 0;
    for (_coord, threats) in threat_map {
        let mut ai_threat_counts: HashMap<ThreatType, usize> = HashMap::new();
        let mut opp_threat_counts: HashMap<ThreatType, usize> = HashMap::new();

        for (threat_type, player) in threats {
            if player == ai_player {
                *ai_threat_counts.entry(threat_type).or_insert(0) += 1;
            } else {
                *opp_threat_counts.entry(threat_type).or_insert(0) += 1;
            }
        }

        // Check AI compound threats
        let ai_l4 = ai_threat_counts
            .get(&ThreatType::LiveFour)
            .copied()
            .unwrap_or(0);
        let ai_d4 = ai_threat_counts
            .get(&ThreatType::DeadFour)
            .copied()
            .unwrap_or(0);
        let ai_l3 = ai_threat_counts
            .get(&ThreatType::LiveThree)
            .copied()
            .unwrap_or(0);

        if ai_l4 > 0 || (ai_d4 > 0 && ai_l3 > 0) {
            ai_compound_score = ai_compound_score.max(SCORE_FOUR_THREE);
        } else if ai_l3 >= 2 {
            ai_compound_score = ai_compound_score.max(SCORE_DOUBLE_LIVE_THREE);
        } else if ai_d4 >= 2 {
            ai_compound_score = ai_compound_score.max(SCORE_DOUBLE_DEAD_FOUR);
        }

        // Check Opponent compound threats (negative score for AI)
        let opp_l4 = opp_threat_counts
            .get(&ThreatType::LiveFour)
            .copied()
            .unwrap_or(0);
        let opp_d4 = opp_threat_counts
            .get(&ThreatType::DeadFour)
            .copied()
            .unwrap_or(0);
        let opp_l3 = opp_threat_counts
            .get(&ThreatType::LiveThree)
            .copied()
            .unwrap_or(0);

        if opp_l4 > 0 || (opp_d4 > 0 && opp_l3 > 0) {
            opp_compound_score = opp_compound_score.max(SCORE_FOUR_THREE);
        } else if opp_l3 >= 2 {
            opp_compound_score = opp_compound_score.max(SCORE_DOUBLE_LIVE_THREE);
        } else if opp_d4 >= 2 {
            opp_compound_score = opp_compound_score.max(SCORE_DOUBLE_DEAD_FOUR);
        }
    }

    // Final score combines base line evaluation and compound threats
    let final_score = (ai_base_score + ai_compound_score) - (opp_base_score + opp_compound_score);
    final_score
}

// Evaluate a single line, return base scores and potential threats at empty spots
fn evaluate_line_detailed(
    line: &[Option<Player>],
    ai: Player,
    opp: Player,
) -> (i32, i32, HashMap<usize, Vec<(ThreatType, Player)>>) {
    let mut ai_score = 0;
    let mut opp_score = 0;
    let mut threats: HashMap<usize, Vec<(ThreatType, Player)>> = HashMap::new();
    let n = line.len();
    let window_size = 6;
    let mut evaluated_mask = vec![false; n]; // Currently unused mask

    for i in 0..=(n.saturating_sub(window_size)) {
        let window = &line[i..(i + window_size)];
        match_patterns(
            window,
            ai,
            opp,
            i,
            &mut ai_score,
            &mut threats,
            &mut evaluated_mask,
        );
        match_patterns(
            window,
            opp,
            ai,
            i,
            &mut opp_score,
            &mut threats,
            &mut evaluated_mask,
        );
    }

    (ai_score, opp_score, threats)
}

// Matches patterns within a 6-cell window
fn match_patterns(
    window: &[Option<Player>],
    player: Player,
    opponent: Player,
    window_start_index: usize,
    score: &mut i32,
    threats: &mut HashMap<usize, Vec<(ThreatType, Player)>>,
    _evaluated_mask: &mut [bool],
) {
    let p = Some(player);
    let o = Some(opponent);
    let n = None;

    // --- Five ---
    if window.windows(5).any(|sub| sub == [p, p, p, p, p]) {
        *score = (*score).max(SCORE_FIVE); // Ensure score reflects win
        return;
    }

    // --- Live Four: N PPPP N ---
    if window == [n, p, p, p, p, n] {
        *score += SCORE_LIVE_FOUR;
        threats
            .entry(window_start_index)
            .or_default()
            .push((ThreatType::LiveFour, player));
        threats
            .entry(window_start_index + 5)
            .or_default()
            .push((ThreatType::LiveFour, player));
        return; // Found Live Four
    }

    // --- Dead Four ---
    let mut is_dead_four = false;
    let mut d4_threat_indices = Vec::new();
    if window[1..5] == [p; 4] {
        // Center PPPPP
        if window[0] == o && window[5] == n {
            is_dead_four = true;
            d4_threat_indices.push(window_start_index + 5);
        } else if window[0] == n && window[5] == o {
            is_dead_four = true;
            d4_threat_indices.push(window_start_index);
        }
    }

    // Fix E0596: Declare closure as mutable
    // Fix warnings: Remove unused start_6, end_6
    let mut check_d4_pattern = |pattern: [Option<Player>; 5], threat_offset: usize| {
        let mut found = false;
        for i in 0..=(window.len() - 5) {
            let sub_window = &window[i..i + 5];
            if sub_window == pattern {
                // Simplified check - just finding the pattern within the 6-window context is enough for D4 threat
                // let start_6 = i.saturating_sub(1); // Removed unused
                // let end_6 = (i + 5).min(window.len()); // Removed unused
                found = true;
                // Mutably borrows d4_threat_indices:
                d4_threat_indices.push(window_start_index + i + threat_offset);
                break; // Found one instance
            }
        }
        found
    };

    is_dead_four |= check_d4_pattern([p, p, p, n, p], 3);
    is_dead_four |= check_d4_pattern([p, p, n, p, p], 2);
    is_dead_four |= check_d4_pattern([p, n, p, p, p], 1);

    if is_dead_four {
        // Avoid double counting score if multiple D4 patterns overlap significantly,
        // but record all distinct threat points.
        // Simple approach: Add score only once if `is_dead_four` is true.
        *score += SCORE_DEAD_FOUR;
        for idx in d4_threat_indices.iter().copied() {
            // Avoid duplicate threat types for the *same player* at the *same location*
            let entry = threats.entry(idx).or_default();
            if !entry.contains(&(ThreatType::DeadFour, player)) {
                entry.push((ThreatType::DeadFour, player));
            }
        }
        // Don't return, allow L3 check for Four-Three
    }

    // --- Live Three ---
    let mut is_live_three = false;
    let mut l3_threat_indices = Vec::new();
    let mut is_jump_l3 = false;
    if window == [n, p, p, p, n, n] {
        is_live_three = true;
        l3_threat_indices.extend([window_start_index, window_start_index + 4]);
    } else if window == [n, n, p, p, p, n] {
        is_live_three = true;
        l3_threat_indices.extend([window_start_index + 1, window_start_index + 5]);
    } else if window == [n, p, n, p, p, n] {
        is_live_three = true;
        is_jump_l3 = true;
        l3_threat_indices.extend([
            window_start_index,
            window_start_index + 2,
            window_start_index + 5,
        ]);
    } else if window == [n, p, p, n, p, n] {
        is_live_three = true;
        is_jump_l3 = true;
        l3_threat_indices.extend([
            window_start_index,
            window_start_index + 3,
            window_start_index + 5,
        ]);
    }

    if is_live_three {
        *score += if is_jump_l3 {
            SCORE_JUMP_LIVE_THREE
        } else {
            SCORE_LIVE_THREE
        };
        for idx in l3_threat_indices.iter().copied() {
            let entry = threats.entry(idx).or_default();
            if !entry.contains(&(ThreatType::LiveThree, player)) {
                entry.push((ThreatType::LiveThree, player));
            }
        }
    }

    // --- Sleepy Three ---
    let mut is_sleepy_three = false;
    let mut s3_threat_indices = Vec::new();
    if window == [o, p, p, p, n, n] {
        is_sleepy_three = true;
        s3_threat_indices.extend([window_start_index + 4, window_start_index + 5]);
    } else if window == [n, n, p, p, p, o] {
        is_sleepy_three = true;
        s3_threat_indices.extend([window_start_index, window_start_index + 1]);
    } else if window == [o, p, p, n, p, n] {
        is_sleepy_three = true;
        s3_threat_indices.extend([window_start_index + 3, window_start_index + 5]);
    } else if window == [n, p, n, p, p, o] {
        is_sleepy_three = true;
        s3_threat_indices.extend([window_start_index, window_start_index + 2]);
    } else if window == [o, p, n, p, p, n] {
        is_sleepy_three = true;
        s3_threat_indices.extend([window_start_index + 2, window_start_index + 5]);
    } else if window == [n, p, p, n, p, o] {
        is_sleepy_three = true;
        s3_threat_indices.extend([window_start_index, window_start_index + 3]);
    } else if window == [n, p, p, p, n, o] {
        is_sleepy_three = true;
        s3_threat_indices.push(window_start_index + 4);
    }
    // Threat at inner N
    else if window == [o, n, p, p, p, n] {
        is_sleepy_three = true;
        s3_threat_indices.push(window_start_index + 1);
    }
    // Threat at inner N
    else if window == [o, p, n, p, n, o] {
        is_sleepy_three = true;
        s3_threat_indices.extend([window_start_index + 2, window_start_index + 4]);
    }

    // Only add sleepy score/threat if not already covered by a live three at this spot
    if is_sleepy_three && !is_live_three {
        *score += SCORE_SLEEPY_THREE;
        for idx in s3_threat_indices.iter().copied() {
            let entry = threats.entry(idx).or_default();
            if !entry.contains(&(ThreatType::SleepyThree, player))
                && !entry.iter().any(|(t, _)| *t == ThreatType::LiveThree)
            {
                // Check no L3 threat exists
                entry.push((ThreatType::SleepyThree, player));
            }
        }
    }

    // --- Live Two & Sleepy Two (Simplified) ---
    // Only check if no higher threats (L3/S3) were found starting in this window region
    if !is_live_three && !is_sleepy_three {
        let mut is_live_two = false;
        let mut l2_threat_indices = Vec::new();
        // N PP NN N...
        if window == [n, p, p, n, n, n] {
            is_live_two = true;
            l2_threat_indices.extend([window_start_index, window_start_index + 3]);
        }
        // Threats at N's
        else if window == [n, n, p, p, n, n] {
            is_live_two = true;
            l2_threat_indices.extend([window_start_index + 1, window_start_index + 4]);
        } else if window == [n, n, n, p, p, n] {
            is_live_two = true;
            l2_threat_indices.extend([window_start_index + 2, window_start_index + 5]);
        }
        // N P NP N N...
        else if window == [n, p, n, p, n, n] {
            is_live_two = true;
            l2_threat_indices.extend([
                window_start_index,
                window_start_index + 2,
                window_start_index + 4,
            ]);
        } else if window == [n, n, p, n, p, n] {
            is_live_two = true;
            l2_threat_indices.extend([
                window_start_index + 1,
                window_start_index + 3,
                window_start_index + 5,
            ]);
        }

        if is_live_two {
            *score += SCORE_LIVE_TWO; // Add Jump L2 score later if needed
            for idx in l2_threat_indices.iter().copied() {
                let entry = threats.entry(idx).or_default();
                if !entry.contains(&(ThreatType::LiveTwo, player)) {
                    entry.push((ThreatType::LiveTwo, player));
                }
            }
        } else {
            // Only check Sleepy Two if Live Two not found
            let mut is_sleepy_two = false;
            let mut s2_threat_indices = Vec::new();
            // O PP NN N...
            if window == [o, p, p, n, n, n] {
                is_sleepy_two = true;
                s2_threat_indices.extend([
                    window_start_index + 3,
                    window_start_index + 4,
                    window_start_index + 5,
                ]);
            }
            // Threats at N's
            else if window == [n, n, n, p, p, o] {
                is_sleepy_two = true;
                s2_threat_indices.extend([
                    window_start_index,
                    window_start_index + 1,
                    window_start_index + 2,
                ]);
            }
            // O P NP N N...
            else if window == [o, p, n, p, n, n] {
                is_sleepy_two = true;
                s2_threat_indices.extend([
                    window_start_index + 2,
                    window_start_index + 4,
                    window_start_index + 5,
                ]);
            } else if window == [n, n, p, n, p, o] {
                is_sleepy_two = true;
                s2_threat_indices.extend([
                    window_start_index,
                    window_start_index + 1,
                    window_start_index + 3,
                ]);
            }
            // N PP NN O...
            else if window == [n, p, p, n, n, o] {
                is_sleepy_two = true;
                s2_threat_indices.extend([
                    window_start_index,
                    window_start_index + 3,
                    window_start_index + 4,
                ]);
            } else if window == [o, n, n, p, p, n] {
                is_sleepy_two = true;
                s2_threat_indices.extend([
                    window_start_index + 1,
                    window_start_index + 2,
                    window_start_index + 5,
                ]);
            }

            if is_sleepy_two {
                *score += SCORE_SLEEPY_TWO;
                for idx in s2_threat_indices.iter().copied() {
                    let entry = threats.entry(idx).or_default();
                    // Avoid adding S2 threat if L2 already exists
                    if !entry.contains(&(ThreatType::SleepyTwo, player))
                        && !entry.iter().any(|(t, _)| *t == ThreatType::LiveTwo)
                    {
                        entry.push((ThreatType::SleepyTwo, player));
                    }
                }
            }
        }
    }
}

// --- ggez EventHandler Implementation ---

impl EventHandler<GameError> for GameState {
    fn update(&mut self, _ctx: &mut Context) -> GameResult {
        if self.phase == GamePhase::Playing
            && self.current_player != self.human_player
            && !self.ai_thinking
        {
            self.trigger_ai_move();
        }
        if self.ai_thinking {
            self.handle_ai_result();
        }
        Ok(())
    }

    fn draw(&mut self, ctx: &mut Context) -> GameResult {
        let mut canvas = graphics::Canvas::from_frame(ctx, Color::from_rgb(200, 150, 100));

        let grid_color = Color::from_rgb(50, 50, 50);
        for i in 0..BOARD_SIZE {
            let y = MARGIN + i as f32 * CELL_SIZE;
            canvas.draw(
                &graphics::Mesh::new_line(
                    ctx,
                    &[Vec2::new(MARGIN, y), Vec2::new(MARGIN + BOARD_DIM, y)],
                    1.0,
                    grid_color,
                )?,
                DrawParam::default(),
            );
            let x = MARGIN + i as f32 * CELL_SIZE;
            canvas.draw(
                &graphics::Mesh::new_line(
                    ctx,
                    &[Vec2::new(x, MARGIN), Vec2::new(x, MARGIN + BOARD_DIM)],
                    1.0,
                    grid_color,
                )?,
                DrawParam::default(),
            );
        }

        let star_points = [(3, 3), (11, 3), (3, 11), (11, 11), (7, 7)];
        for (x, y) in star_points {
            if x < BOARD_SIZE && y < BOARD_SIZE {
                let pos = self.grid_to_screen(x, y);
                canvas.draw(
                    &graphics::Mesh::new_circle(
                        ctx,
                        DrawMode::fill(),
                        pos,
                        DOT_RADIUS,
                        0.1,
                        grid_color,
                    )?,
                    DrawParam::default(),
                );
            }
        }

        for r in 0..BOARD_SIZE {
            for c in 0..BOARD_SIZE {
                if let Some(player) = self.board[r][c] {
                    let pos = self.grid_to_screen(c, r);
                    canvas.draw(
                        &graphics::Mesh::new_circle(
                            ctx,
                            DrawMode::fill(),
                            pos,
                            PIECE_RADIUS,
                            0.1,
                            player.color(),
                        )?,
                        DrawParam::default(),
                    );
                }
            }
        }

        if let Some((x, y)) = self.last_move {
            let pos = self.grid_to_screen(x, y);
            let highlight_color = Color::new(1.0, 0.0, 0.0, 0.8);
            let rect = Rect::new(
                pos.x - HIGHLIGHT_SIZE / 2.0,
                pos.y - HIGHLIGHT_SIZE / 2.0,
                HIGHLIGHT_SIZE,
                HIGHLIGHT_SIZE,
            );
            canvas.draw(
                &graphics::Mesh::new_rectangle(ctx, DrawMode::stroke(2.0), rect, highlight_color)?,
                DrawParam::default(),
            );
        }

        let text = Text::new(TextFragment {
            text: self.status_text.clone(),
            color: Some(Color::BLACK),
            font: Some("LiberationMono-Regular".into()),
            scale: Some(graphics::PxScale::from(20.0)),
            ..Default::default()
        });
        let text_pos = Vec2::new(MARGIN, WINDOW_HEIGHT - MARGIN / 2.0 - 10.0);
        canvas.draw(&text, DrawParam::default().dest(text_pos));

        canvas.finish(ctx)?;
        Ok(())
    }

    fn mouse_button_down_event(
        &mut self,
        _ctx: &mut Context,
        button: MouseButton,
        x: f32,
        y: f32,
    ) -> GameResult {
        if button == MouseButton::Left {
            if self.phase != GamePhase::Playing {
                return Ok(());
            }
            if self.ai_thinking || self.current_player != self.human_player {
                return Ok(());
            }

            if let Some((grid_x, grid_y)) = self.screen_to_grid(x, y) {
                match self.make_move(grid_x, grid_y, self.human_player) {
                    Ok(_) => println!(
                        "Player ({:?}) placed at ({}, {})",
                        self.human_player, grid_x, grid_y
                    ),
                    Err(e) => println!("Invalid move: {}", e),
                }
            }
        }
        Ok(())
    }

    fn key_down_event(
        &mut self,
        ctx: &mut Context,
        input: ggez::input::keyboard::KeyInput,
        _repeated: bool,
    ) -> GameResult {
        match input.keycode {
            Some(ggez::input::keyboard::KeyCode::U) => {
                if self.phase == GamePhase::Playing && !self.ai_thinking {
                    self.undo_move();
                } else {
                    println!("Cannot undo now.");
                }
            }
            Some(ggez::input::keyboard::KeyCode::R) => {
                self.reset_game();
            }
            Some(ggez::input::keyboard::KeyCode::B) => {
                if self.phase == GamePhase::ChooseSide {
                    self.human_player = Player::Black;
                    self.phase = GamePhase::Playing;
                    println!("You chose Black. Game started.");
                    self.update_status_text();
                }
            }
            Some(ggez::input::keyboard::KeyCode::W) => {
                if self.phase == GamePhase::ChooseSide {
                    self.human_player = Player::White;
                    self.phase = GamePhase::Playing;
                    println!("You chose White. Game started. AI (Black) moves first.");
                    self.update_status_text();
                }
            }
            Some(ggez::input::keyboard::KeyCode::Escape) => {
                ctx.request_quit();
            }
            _ => {}
        }
        Ok(())
    }
}

// --- Main Function ---
fn main() -> GameResult {
    let (mut ctx, event_loop) = ContextBuilder::new("gomoku_ggez_ai", "Gomoku AI")
        .window_setup(WindowSetup::default().title("Gomoku"))
        .window_mode(
            WindowMode::default()
                .dimensions(WINDOW_WIDTH, WINDOW_HEIGHT)
                .resizable(false),
        )
        .build()?;

    let state = GameState::new(&mut ctx)?;
    event::run(ctx, event_loop, state)
}
