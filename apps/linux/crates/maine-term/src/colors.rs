use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

/// The default theme. Colors are `0xRRGGBB`.
#[derive(Debug, Clone)]
pub struct Palette {
    pub ansi: [u32; 16],
    pub foreground: u32,
    pub background: u32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            ansi: [
                0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7,
                0xc5c8c6, // normal
                0x666666, 0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1,
                0xeaeaea, // bright
            ],
            foreground: 0xd4d4d4,
            background: 0x1e1e1e,
        }
    }
}

fn pack(rgb: Rgb) -> u32 {
    (rgb.r as u32) << 16 | (rgb.g as u32) << 8 | rgb.b as u32
}

impl Palette {
    fn indexed(&self, i: u8) -> u32 {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let n = i as u32 - 16;
                let level = |v: u32| if v == 0 { 0 } else { 55 + v * 40 };
                level(n / 36) << 16 | level(n / 6 % 6) << 8 | level(n % 6)
            }
            _ => {
                let g = 8 + (i as u32 - 232) * 10;
                g << 16 | g << 8 | g
            }
        }
    }

    fn named(&self, n: NamedColor) -> u32 {
        use NamedColor::*;
        let dim = |v: u32| {
            let (r, g, b) = ((v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
            (r * 2 / 3) << 16 | (g * 2 / 3) << 8 | (b * 2 / 3)
        };
        match n {
            Foreground | BrightForeground | Cursor => self.foreground,
            Background => self.background,
            DimForeground => dim(self.foreground),
            DimBlack => dim(self.ansi[0]),
            DimRed => dim(self.ansi[1]),
            DimGreen => dim(self.ansi[2]),
            DimYellow => dim(self.ansi[3]),
            DimBlue => dim(self.ansi[4]),
            DimMagenta => dim(self.ansi[5]),
            DimCyan => dim(self.ansi[6]),
            DimWhite => dim(self.ansi[7]),
            other => self.ansi[(other as usize).min(15)],
        }
    }

    /// Resolves a cell color to `0xRRGGBB`, honoring palette overrides the
    /// running program set with OSC 4/10/11.
    pub fn resolve(&self, color: Color, overrides: &Colors) -> u32 {
        match color {
            Color::Spec(rgb) => pack(rgb),
            Color::Indexed(i) => overrides[i as usize]
                .map(pack)
                .unwrap_or_else(|| self.indexed(i)),
            Color::Named(n) => overrides[n as usize]
                .map(pack)
                .unwrap_or_else(|| self.named(n)),
        }
    }
}
