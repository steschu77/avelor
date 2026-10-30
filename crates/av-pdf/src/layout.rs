//! Placed glyphs into lines and lines into text: lines by their baselines,
//! words by the gaps between glyphs, paragraphs by the gaps between lines.
//! Columns side by side read as one line; their order is left out.

// ----------------------------------------------------------------------------
/// A line of text and its extent on the page, y growing downwards (any
/// unit).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub top: f64,
    pub bottom: f64,
}

// ----------------------------------------------------------------------------
/// A glyph where it was placed, y growing downwards.
#[derive(Clone, Debug)]
pub(crate) struct Placed {
    pub x: f64,
    /// Where its advance ends.
    pub end: f64,
    pub baseline: f64,
    pub size: f64,
    pub text: String,
}

// ----------------------------------------------------------------------------
/// The glyphs as lines, top to bottom. A glyph printed again over itself
/// (fake bold) is dropped; a gap wider than a fifth of the size is a space.
pub(crate) fn lines(mut glyphs: Vec<Placed>) -> Vec<Line> {
    glyphs.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.x.total_cmp(&b.x)));
    let mut groups: Vec<Vec<Placed>> = Vec::new();
    for glyph in glyphs {
        let joins = groups.last().is_some_and(|line| {
            let size = line.iter().map(|g| g.size).fold(glyph.size, f64::max);
            (glyph.baseline - line[0].baseline).abs() <= 0.5 * size
        });
        match groups.last_mut() {
            Some(line) if joins => line.push(glyph),
            _ => groups.push(vec![glyph]),
        }
    }

    let mut lines = Vec::new();
    for mut group in groups {
        group.sort_by(|a, b| a.x.total_cmp(&b.x));
        let size = group.iter().map(|g| g.size).fold(0.0, f64::max);
        let baseline = group[0].baseline;
        let mut text = String::new();
        let mut last: Option<&Placed> = None;
        for glyph in &group {
            if let Some(prev) = last {
                let over = prev.text == glyph.text && (glyph.x - prev.x).abs() < 0.1 * glyph.size;
                if over {
                    continue;
                }
                let gap = glyph.x - prev.end;
                if gap > 0.2 * glyph.size.max(prev.size)
                    && !text.ends_with(' ')
                    && !glyph.text.starts_with(' ')
                {
                    text.push(' ');
                }
            }
            text.push_str(&glyph.text);
            last = Some(glyph);
        }
        let text = text.trim().to_string();
        if !text.is_empty() {
            lines.push(Line {
                text,
                top: baseline - 0.8 * size,
                bottom: baseline + 0.2 * size,
            });
        }
    }
    lines
}

// ----------------------------------------------------------------------------
/// The lines, in their order, as text: a blank line between two whose gap
/// is wider than most of a line's height.
pub fn to_text(lines: &[Line]) -> String {
    let mut heights: Vec<f64> = lines.iter().map(|l| l.bottom - l.top).collect();
    heights.sort_by(f64::total_cmp);
    let height = heights.get(heights.len() / 2).copied().unwrap_or(0.0);
    let mut text = String::new();
    let mut prev: Option<&Line> = None;
    for line in lines.iter().filter(|l| !l.text.trim().is_empty()) {
        if let Some(prev) = prev {
            text.push('\n');
            if line.top - prev.bottom > 0.8 * height {
                text.push('\n');
            }
        }
        text.push_str(line.text.trim_end());
        prev = Some(line);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    fn word(x: f64, baseline: f64, text: &str) -> Vec<Placed> {
        let size = 10.0;
        text.chars()
            .enumerate()
            .map(|(i, c)| Placed {
                x: x + i as f64 * 5.0,
                end: x + (i + 1) as f64 * 5.0,
                baseline,
                size,
                text: c.to_string(),
            })
            .collect()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn glyphs_make_lines_words_and_paragraphs() {
        let mut glyphs = Vec::new();
        // Shown out of order: the second line first.
        glyphs.extend(word(10.0, 112.0, "EUR"));
        glyphs.extend(word(10.0, 100.0, "Rechnung"));
        glyphs.extend(word(60.0, 100.0, "42"));
        // A superscript, a little higher on the first line.
        glyphs.extend(word(70.0, 97.0, "1"));
        glyphs.extend(word(10.0, 140.0, "Danke"));
        // Fake bold: the same glyphs again, a little to the right.
        glyphs.extend(word(10.3, 140.0, "Danke"));
        let lines = lines(glyphs);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Rechnung 421", "EUR", "Danke"]);
        assert_eq!(to_text(&lines), "Rechnung 421\nEUR\n\nDanke");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn spaces_in_the_text_are_not_doubled() {
        let mut glyphs = word(0.0, 10.0, "a ");
        glyphs.extend(word(20.0, 10.0, "b"));
        assert_eq!(lines(glyphs)[0].text, "a b");
    }
}
