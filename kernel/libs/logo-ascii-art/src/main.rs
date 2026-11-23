use logo_ascii_art::get_black_white_version;

fn gen_gradient_color_version() -> String {
    use owo_colors::{Rgb, *};

    let start_gradient_color = Rgb(33, 54, 245);
    let end_gradient_color = Rgb(113, 240, 252);
    let colored_logo = apply_gradient(
        get_black_white_version(),
        start_gradient_color,
        end_gradient_color,
    );
    return colored_logo;

    fn apply_gradient(ascii_art: &str, start_color: Rgb, end_color: Rgb) -> String {
        let lines: Vec<&str> = ascii_art.lines().collect();
        if lines.is_empty() {
            return String::new();
        }

        let interpolate = |col| -> Rgb {
            let min_col = lines
                .iter()
                .flat_map(|line| line.char_indices())
                .filter(|(_, c)| !c.is_whitespace())
                .map(|(idx, _)| idx)
                .min()
                .unwrap_or(0);

            let max_col = lines
                .iter()
                .flat_map(|line| line.char_indices())
                .filter(|(_, c)| !c.is_whitespace())
                .map(|(idx, _)| idx)
                .max()
                .unwrap_or(0);

            assert!(min_col != max_col);

            if col < min_col {
                return start_color;
            }
            if col > max_col {
                return end_color;
            }

            let r = start_color.0 as f32
                + (end_color.0 as f32 - start_color.0 as f32) * (col - min_col) as f32
                    / (max_col - min_col) as f32;
            let g = start_color.1 as f32
                + (end_color.1 as f32 - start_color.1 as f32) * (col - min_col) as f32
                    / (max_col - min_col) as f32;
            let b = start_color.2 as f32
                + (end_color.2 as f32 - start_color.2 as f32) * (col - min_col) as f32
                    / (max_col - min_col) as f32;

            Rgb(r as u8, g as u8, b as u8)
        };

        let mut result = String::new();
        for line in &lines {
            for (col, ch) in line.chars().enumerate() {
                if ch.is_whitespace() {
                    result.push(ch);
                } else {
                    let color = interpolate(col);
                    result.push_str(&ch.color(color).to_string());
                }
            }
            result.push('\n');
        }

        result
    }
}

fn main() {
    print!("{}", gen_gradient_color_version());
}
