//! Paragraph-opening markers of Korean public documents (D25), read from
//! the source text alone: 제N장 and Ⅰ. open a chapter, 제N절 a section, and
//! the customary four list levels are □, ○ (or ㅇ), a dash and a middle dot.

/// The heading level (2 for 제N장 and Ⅰ., 3 for 제N절) a paragraph's text
/// opens with. A heading must be one short line: the contents page's
/// "제1장 …" paragraph goes on with line breaks and tabs.
pub fn chapter_level(text: &str) -> Option<u32> {
    let one_line = !text.contains(['\n', '\t']);
    let roman = |c: char| ('\u{2160}'..='\u{216b}').contains(&c);
    let mut chars = text.chars();
    let first = chars.next()?;
    let rest = chars.as_str();
    if first == '제' {
        let number = rest.trim_start();
        let after = number.trim_start_matches(|c: char| c.is_ascii_digit());
        if after.len() == number.len() {
            return None;
        }
        return match after.trim_start().chars().next() {
            Some('장') => one_line.then_some(2),
            Some('절') => one_line.then_some(3),
            _ => None,
        };
    }
    if roman(first) {
        let after = rest.trim_start_matches(roman);
        return (one_line && after.starts_with(['.', '．'])).then_some(2);
    }
    None
}

/// The list level a paragraph's opening marker stands for: □ (or a symbol
/// font's private-use glyph in its place, such as 0713AI 도입's circled
/// numbers), ○, a dash and a middle dot.
pub fn marker_rank(text: &str) -> Option<u32> {
    let mut chars = text.chars();
    let first = chars.next()?;
    let rest = chars.as_str();
    let spaced = rest.starts_with(char::is_whitespace);
    match first {
        '□' | '■' => Some(0),
        _ if spaced && is_private_use(first) => Some(0),
        '○' | '●' | '◦' => Some(1),
        // The Hangul letter ㅇ commonly stands in for ○.
        'ㅇ' if spaced => Some(1),
        // "- 정보관리담당관('26.7.16) -" is a byline between dashes.
        '-' | '–' | '—' if spaced && !rest.trim_end().ends_with(['-', '–', '—']) => Some(2),
        '·' | 'ㆍ' | '•' | '∙' => Some(3),
        _ => None,
    }
}

fn is_private_use(c: char) -> bool {
    matches!(u32::from(c), 0xE000..=0xF8FF | 0xF0000..=0x10FFFD)
}
