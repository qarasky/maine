use crate::colors::Palette;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::CursorShape as VteCursorShape;

pub use alacritty_terminal::vte::ansi::CursorShape;

/// Per-cell attribute bits handed to the painter.
pub struct CellFlags;
impl CellFlags {
    pub const BOLD: u16 = 1;
    pub const ITALIC: u16 = 2;
    pub const UNDERLINE: u16 = 4;
    pub const STRIKEOUT: u16 = 8;
    /// First column of a double-width character.
    pub const WIDE: u16 = 16;
    /// Second column of a double-width character: draw nothing.
    pub const SPACER: u16 = 32;
    pub const SELECTED: u16 = 64;
    pub const CURSOR: u16 = 128;
}

/// One resolved screen cell. Colors are `0xRRGGBB` with inverse, dim and
/// hidden already applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: u32,
    pub bg: u32,
    pub flags: u16,
}

/// Terminal modes the UI needs for input handling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modes {
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub bracketed_paste: bool,
    pub alt_screen: bool,
    pub focus_reporting: bool,
    pub mouse_click: bool,
    pub mouse_drag: bool,
    pub mouse_motion: bool,
    pub sgr_mouse: bool,
    pub alternate_scroll: bool,
}

impl Modes {
    pub fn mouse_reporting(&self) -> bool {
        self.mouse_click || self.mouse_drag || self.mouse_motion
    }

    pub(crate) fn from(mode: TermMode) -> Self {
        Self {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            app_keypad: mode.contains(TermMode::APP_KEYPAD),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            focus_reporting: mode.contains(TermMode::FOCUS_IN_OUT),
            mouse_click: mode.contains(TermMode::MOUSE_REPORT_CLICK),
            mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
            mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
        }
    }
}

/// The visible screen, row-major, `cols * rows` cells.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    /// `(col, row, shape)` of a visible cursor, if any.
    pub cursor: Option<(usize, usize, CursorShape)>,
    /// Lines scrolled back from the bottom (0 = live).
    pub display_offset: usize,
    /// Scrollback lines available above the screen.
    pub history: usize,
    pub modes: Modes,
    pub default_fg: u32,
    pub default_bg: u32,
}

impl Snapshot {
    pub fn row_text(&self, row: usize) -> String {
        let start = row * self.cols;
        let text: String = self.cells[start..start + self.cols]
            .iter()
            .filter(|c| c.flags & CellFlags::SPACER == 0)
            .map(|c| c.ch)
            .collect();
        text.trim_end().to_string()
    }

    pub fn text(&self) -> String {
        let lines: Vec<String> = (0..self.rows).map(|r| self.row_text(r)).collect();
        lines.join("\n").trim_end().to_string()
    }
}

pub(crate) fn snapshot<T: alacritty_terminal::event::EventListener>(
    term: &Term<T>,
    palette: &Palette,
) -> Snapshot {
    let cols = term.columns();
    let rows = term.screen_lines();
    let content = term.renderable_content();
    let offset = content.display_offset;
    let mut cells = vec![
        Cell {
            ch: ' ',
            fg: palette.foreground,
            bg: palette.background,
            flags: 0
        };
        cols * rows
    ];
    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset as i32;
        let col = indexed.point.column.0;
        if row < 0 || row as usize >= rows || col >= cols {
            continue;
        }
        let c = &indexed.cell;
        let mut fg = palette.resolve(c.fg, content.colors);
        let mut bg = palette.resolve(c.bg, content.colors);
        if c.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if c.flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let mut flags = 0u16;
        if c.flags.intersects(Flags::BOLD) {
            flags |= CellFlags::BOLD;
        }
        if c.flags.contains(Flags::ITALIC) {
            flags |= CellFlags::ITALIC;
        }
        if c.flags.intersects(Flags::ALL_UNDERLINES) {
            flags |= CellFlags::UNDERLINE;
        }
        if c.flags.contains(Flags::STRIKEOUT) {
            flags |= CellFlags::STRIKEOUT;
        }
        if c.flags.contains(Flags::WIDE_CHAR) {
            flags |= CellFlags::WIDE;
        }
        if c.flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            flags |= CellFlags::SPACER;
        }
        if let Some(sel) = &content.selection {
            if sel.contains(indexed.point) {
                flags |= CellFlags::SELECTED;
            }
        }
        cells[row as usize * cols + col] = Cell {
            ch: c.c,
            fg,
            bg,
            flags,
        };
    }
    let cursor = if content.mode.contains(TermMode::SHOW_CURSOR)
        && offset == 0
        && content.cursor.shape != VteCursorShape::Hidden
    {
        let (col, line) = (content.cursor.point.column.0, content.cursor.point.line.0);
        (line >= 0 && (line as usize) < rows && col < cols).then_some((
            col,
            line as usize,
            content.cursor.shape,
        ))
    } else {
        None
    };
    if let Some((col, row, _)) = cursor {
        cells[row * cols + col].flags |= CellFlags::CURSOR;
    }
    Snapshot {
        cols,
        rows,
        cells,
        cursor,
        display_offset: offset,
        history: term.history_size(),
        modes: Modes::from(content.mode),
        default_fg: palette.foreground,
        default_bg: palette.background,
    }
}
