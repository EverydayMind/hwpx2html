//! A Hancom equation script as MathML (user decision Q1, plan §3.1.3).
//!
//! Only the syntax the corpus uses is converted: words and numbers, `{...}`
//! groups, the thin space written as a backtick, `A over B`, `sqrt X`, `sum`
//! with `_` and `^`, `TIMES`, and the plain marks `= + - * % ( )` and a few
//! more. A script that uses anything else (a command this converter does not
//! know, unbalanced braces) is not converted: the caller writes its letters
//! as they are, so the equation's words never go missing and a command is never
//! guessed at.
//!
//! The script is written once, in the semantic tree. The MathML carries its
//! words, digits and marks in script order; commands become structure
//! (`mfrac`, `msqrt`, `msub`, ...) or their sign (`TIMES` is `×`).

/// Width of one backtick, in em: the reference draws it a quarter em wide
/// (in 1.산업기술혁신사업 공통운영요령's equation images, single spaces are a
/// quarter of the 10pt body, double ones twice that).
const THIN_SPACE: f64 = 0.25;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Open,
    Close,
    Sub,
    Sup,
    /// A run of this many backticks.
    Space(usize),
    Over,
    Times,
    Sqrt,
    Sum,
    /// A word of letters (Hangul, or one Latin letter).
    Word(String),
    Number(String),
    Mark(char),
}

/// Marks written as themselves: plain characters the script puts between
/// words. A mark not listed here is a letter of a word.
const MARKS: &str = "=+-*%(),.:;!<>|/";

fn tokenize(script: &str) -> Option<Vec<Token>> {
    let chars = script.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        match c {
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '_' => tokens.push(Token::Sub),
            '^' => tokens.push(Token::Sup),
            '`' => {
                let start = at;
                while chars.get(at + 1) == Some(&'`') {
                    at += 1;
                }
                tokens.push(Token::Space(at - start + 1));
            }
            c if c.is_whitespace() => {}
            c if c.is_ascii_digit() => {
                let start = at;
                while chars.get(at + 1).is_some_and(|c| c.is_ascii_digit()) {
                    at += 1;
                }
                // A decimal point followed by a digit belongs to the number.
                if chars.get(at + 1) == Some(&'.')
                    && chars.get(at + 2).is_some_and(|c| c.is_ascii_digit())
                {
                    at += 1;
                    while chars.get(at + 1).is_some_and(|c| c.is_ascii_digit()) {
                        at += 1;
                    }
                }
                tokens.push(Token::Number(chars[start..=at].iter().collect()));
            }
            c if MARKS.contains(c) => tokens.push(Token::Mark(c)),
            _ => {
                let start = at;
                let is_letter = |c: char| {
                    !c.is_whitespace()
                        && !c.is_ascii_digit()
                        && !MARKS.contains(c)
                        && !matches!(c, '{' | '}' | '_' | '^' | '`')
                };
                while chars.get(at + 1).is_some_and(|c| is_letter(*c)) {
                    at += 1;
                }
                let word = chars[start..=at].iter().collect::<String>();
                tokens.push(match word.to_ascii_lowercase().as_str() {
                    "over" => Token::Over,
                    "times" => Token::Times,
                    "sqrt" => Token::Sqrt,
                    "sum" => Token::Sum,
                    // A Latin word of more than one letter is a command this
                    // converter does not know (`alpha`, `LEFT`, `rm`, ...),
                    // not text.
                    _ if word.len() > 1 && word.chars().all(|c| c.is_ascii_alphabetic()) => {
                        return None
                    }
                    _ => Token::Word(word),
                });
            }
        }
        at += 1;
    }
    Some(tokens)
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Text(String),
    Ident(String),
    Number(String),
    Mark(char),
    Times,
    Space(usize),
    Sum,
    Row(Vec<Node>),
    Fraction(Box<Node>, Box<Node>),
    Sqrt(Box<Node>),
    Script {
        base: Box<Node>,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
    },
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    /// The items up to the `}` that closes the group (when `nested`) or the
    /// end of the script. `A over B` and `X _Y` take the item before them.
    fn row(&mut self, nested: bool) -> Option<Node> {
        let mut items: Vec<Node> = Vec::new();
        loop {
            match self.peek() {
                None => return if nested { None } else { Some(Node::Row(items)) },
                Some(Token::Close) => {
                    if !nested {
                        return None;
                    }
                    self.next();
                    return Some(Node::Row(items));
                }
                Some(Token::Over) => {
                    self.next();
                    let right = self.item()?;
                    let left = items.pop()?;
                    items.push(Node::Fraction(Box::new(left), Box::new(right)));
                }
                Some(Token::Sub | Token::Sup) => {
                    let superscript = self.next() == Some(Token::Sup);
                    let operand = Box::new(self.item()?);
                    let base = items.pop()?;
                    items.push(attach(base, operand, superscript)?);
                }
                Some(_) => items.push(self.item()?),
            }
        }
    }

    /// One operand: a group, a word, a number, a mark or a `sqrt` of one.
    fn item(&mut self) -> Option<Node> {
        Some(match self.next()? {
            Token::Open => self.row(true)?,
            Token::Word(word) if word.chars().count() == 1 && word.is_ascii() => Node::Ident(word),
            Token::Word(word) => Node::Text(word),
            Token::Number(digits) => Node::Number(digits),
            Token::Mark(mark) => Node::Mark(mark),
            Token::Space(count) => Node::Space(count),
            Token::Times => Node::Times,
            Token::Sum => Node::Sum,
            Token::Sqrt => Node::Sqrt(Box::new(self.item()?)),
            Token::Close | Token::Over | Token::Sub | Token::Sup => return None,
        })
    }
}

/// `base _sub` or `base ^sup`, joined with the other script when it is
/// already there. A second script of the same kind is not converted.
fn attach(base: Node, operand: Box<Node>, superscript: bool) -> Option<Node> {
    match base {
        Node::Script { base, sub, sup } => {
            let (sub, sup) = if superscript {
                sup.is_none().then_some(())?;
                (sub, Some(operand))
            } else {
                sub.is_none().then_some(())?;
                (Some(operand), sup)
            };
            Some(Node::Script { base, sub, sup })
        }
        base => Some(Node::Script {
            base: Box::new(base),
            sub: (!superscript).then_some(operand.clone()),
            sup: superscript.then_some(operand),
        }),
    }
}

fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

fn leaf(tag: &str, text: &str, out: &mut String) {
    out.push('<');
    out.push_str(tag);
    out.push('>');
    escape(text, out);
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// An operand as one `mrow` (a group's items, or the item itself) carrying
/// `attributes`. `at_text_size` is as for [`write`].
fn operand(node: &Node, attributes: &str, at_text_size: bool, out: &mut String) {
    out.push_str("<mrow");
    out.push_str(attributes);
    out.push('>');
    match node {
        Node::Row(items) => {
            for item in items {
                write(item, at_text_size, out);
            }
        }
        _ => write(node, at_text_size, out),
    }
    out.push_str("</mrow>");
}

/// `node` as MathML. `at_text_size` is whether it is set at the size of
/// the text around the equation: not inside a script or a fraction.
fn write(node: &Node, at_text_size: bool, out: &mut String) {
    match node {
        Node::Text(text) => leaf("mtext", text, out),
        Node::Ident(letter) => leaf("mi", letter, out),
        Node::Number(digits) => leaf("mn", digits, out),
        // A `*` is the asterisk as written, not the operator the dictionary
        // maps it to; the other marks are operators in their own spacing.
        Node::Mark('*') => leaf("mtext", "*", out),
        Node::Mark(mark) => leaf("mo", &mark.to_string(), out),
        Node::Times => leaf("mo", "\u{d7}", out),
        Node::Space(count) => out.push_str(&format!(
            "<mspace width=\"{}em\"></mspace>",
            THIN_SPACE * *count as f64
        )),
        Node::Sum => leaf("mo", "\u{2211}", out),
        Node::Row(items) => {
            out.push_str("<mrow>");
            for item in items {
                write(item, at_text_size, out);
            }
            out.push_str("</mrow>");
        }
        Node::Fraction(top, bottom) => {
            // The reference draws the parts of a fraction in the text at the
            // text's size, where MathML's inline (compact) fraction would set
            // them one script level smaller: script level 0 keeps them at it.
            // A fraction inside another fraction or a script keeps MathML's
            // rule; the corpus has none to show Hancom's.
            let level = if at_text_size {
                " scriptlevel=\"0\""
            } else {
                ""
            };
            out.push_str("<mfrac>");
            operand(top, level, false, out);
            operand(bottom, level, false, out);
            out.push_str("</mfrac>");
        }
        Node::Sqrt(inside) => {
            out.push_str("<msqrt>");
            match inside.as_ref() {
                Node::Row(items) => {
                    for item in items {
                        write(item, at_text_size, out);
                    }
                }
                other => write(other, at_text_size, out),
            }
            out.push_str("</msqrt>");
        }
        Node::Script { base, sub, sup } => {
            // A sum's limits stand above and below it, whatever the size of
            // the text around it.
            let sum = matches!(base.as_ref(), Node::Sum);
            let tag = match (sub.is_some(), sup.is_some(), sum) {
                (true, true, false) => "msubsup",
                (true, false, false) => "msub",
                (false, _, false) => "msup",
                (true, true, true) => "munderover",
                (true, false, true) => "munder",
                (false, _, true) => "mover",
            };
            if sum {
                out.push_str("<mstyle displaystyle=\"true\">");
            }
            out.push('<');
            out.push_str(tag);
            out.push('>');
            operand(base, "", at_text_size, out);
            if let Some(sub) = sub {
                operand(sub, "", false, out);
            }
            if let Some(sup) = sup {
                operand(sup, "", false, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
            if sum {
                out.push_str("</mstyle>");
            }
        }
    }
}

/// The script as the content of a `math` element (`<mrow>...</mrow>`), or
/// `None` for a script with syntax outside the converted subset.
pub fn to_mathml(script: &str) -> Option<String> {
    let mut parser = Parser {
        tokens: tokenize(script)?,
        at: 0,
    };
    let row = parser.row(false)?;
    let mut out = String::new();
    write(&row, true, &mut out);
    Some(out)
}

/// The text a converted equation shows, in script order, without anything
/// the converter turned into structure or spacing: what a reader of the
/// page can check against the script. `TIMES` is its sign.
pub fn shown_text(script: &str) -> Option<String> {
    fn collect(node: &Node, out: &mut String) {
        match node {
            Node::Text(text) | Node::Ident(text) | Node::Number(text) => out.push_str(text),
            Node::Mark(mark) => out.push(*mark),
            Node::Times => out.push('\u{d7}'),
            Node::Sum => out.push('\u{2211}'),
            Node::Space(_) => {}
            Node::Row(items) => items.iter().for_each(|item| collect(item, out)),
            Node::Fraction(top, bottom) => {
                collect(top, out);
                collect(bottom, out);
            }
            Node::Sqrt(inside) => collect(inside, out),
            Node::Script { base, sub, sup } => {
                collect(base, out);
                if let Some(sub) = sub {
                    collect(sub, out);
                }
                if let Some(sup) = sup {
                    collect(sup, out);
                }
            }
        }
    }
    let mut parser = Parser {
        tokens: tokenize(script)?,
        at: 0,
    };
    let row = parser.row(false)?;
    let mut out = String::new();
    collect(&row, &mut out);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mathml(script: &str) -> String {
        to_mathml(script).unwrap_or_else(|| panic!("{script}"))
    }

    #[test]
    fn a_fraction_takes_the_group_before_and_after_it() {
        assert_eq!(
            mathml("{a} over {2000-8}"),
            "<mrow><mfrac><mrow scriptlevel=\"0\"><mi>a</mi></mrow><mrow scriptlevel=\"0\"><mn>2000</mn><mo>-</mo><mn>8</mn></mrow></mfrac></mrow>"
        );
    }

    #[test]
    fn only_the_parts_of_a_fraction_in_the_text_keep_its_size() {
        // A fraction in the text, here under a square root as in the
        // 성과보고서's FSI: both parts at script level 0, and an exponent in
        // them still a level smaller by MathML's own rule.
        assert_eq!(
            mathml("sqrt {{a TIMES 10 ^{6}} over {b}}"),
            "<mrow><msqrt><mfrac><mrow scriptlevel=\"0\"><mi>a</mi><mo>\u{d7}</mo><msup><mrow><mn>10</mn></mrow><mrow><mn>6</mn></mrow></msup></mrow><mrow scriptlevel=\"0\"><mi>b</mi></mrow></mfrac></msqrt></mrow>"
        );
        // A fraction inside a fraction or a script keeps MathML's rule.
        assert_eq!(
            mathml("{{a} over {b}} over {c}"),
            "<mrow><mfrac><mrow scriptlevel=\"0\"><mfrac><mrow><mi>a</mi></mrow><mrow><mi>b</mi></mrow></mfrac></mrow><mrow scriptlevel=\"0\"><mi>c</mi></mrow></mfrac></mrow>"
        );
        assert!(!mathml("x ^{{a} over {b}}").contains("scriptlevel"));
        // A sum's limits are scripts, not fraction parts.
        assert!(!mathml("sum _{i=1} ^{n}").contains("scriptlevel"));
    }

    #[test]
    fn over_binds_to_its_neighbours_before_times() {
        // ({A} over {B}) TIMES 100 and {A} over {B} TIMES {C} over {D}: a
        // fraction is one item of the row.
        let row = mathml("{가} over {나} TIMES {다} over {라}");
        assert_eq!(row.matches("<mfrac>").count(), 2, "{row}");
        let times = row.find('\u{d7}').unwrap();
        assert!(row[..times].contains("</mfrac>"), "{row}");
        assert!(row[times..].contains("<mfrac>"), "{row}");
    }

    #[test]
    fn thin_spaces_are_a_quarter_em_each_and_blanks_are_nothing() {
        let row = mathml("{a`b``c} d");
        assert!(row.contains("<mspace width=\"0.25em\"></mspace>"), "{row}");
        assert!(row.contains("<mspace width=\"0.5em\"></mspace>"), "{row}");
        // The blank before `d` is only a separator.
        assert_eq!(row.matches("<mspace").count(), 2, "{row}");
    }

    #[test]
    fn words_numbers_and_marks_keep_their_characters() {
        let row = mathml("*``가동률= 87.5%");
        assert!(row.contains("<mtext>*</mtext>"), "{row}");
        assert!(row.contains("<mtext>가동률</mtext>"), "{row}");
        assert!(row.contains("<mo>=</mo>"), "{row}");
        assert!(row.contains("<mn>87.5</mn>"), "{row}");
        assert!(row.contains("<mo>%</mo>"), "{row}");
        // A number followed by letters is two tokens.
        assert_eq!(
            shown_text("(4대보험`본인부담포함)").unwrap(),
            "(4대보험본인부담포함)"
        );
    }

    #[test]
    fn a_square_root_holds_its_group() {
        assert_eq!(
            mathml("sqrt {도수율 TIMES 강도율}"),
            "<mrow><msqrt><mtext>도수율</mtext><mo>\u{d7}</mo><mtext>강도율</mtext></msqrt></mrow>"
        );
    }

    #[test]
    fn scripts_attach_to_the_item_before_and_join() {
        assert_eq!(
            mathml("10 ^{6}"),
            "<mrow><msup><mrow><mn>10</mn></mrow><mrow><mn>6</mn></mrow></msup></mrow>"
        );
        // Both scripts of a sum make its limits, above and below in any size.
        assert_eq!(
            mathml("sum _{i=1} ^{n}"),
            "<mrow><mstyle displaystyle=\"true\"><munderover><mrow><mo>\u{2211}</mo></mrow><mrow><mi>i</mi><mo>=</mo><mn>1</mn></mrow><mrow><mi>n</mi></mrow></munderover></mstyle></mrow>"
        );
        assert!(mathml("x _{1} ^{2}").contains("<msubsup>"));
        assert!(mathml("x _{1}").contains("<msub>"));
    }

    #[test]
    fn the_corpus_scripts_all_convert_and_show_their_words() {
        for (script, shown) in [
            (
                "*``인건비계상률=`( {해당`연(월)`참여연구자``지급`급여(4대보험`본인부담포함)} over {해당`연(월)`급여`총액} )",
                "*인건비계상률=(해당연(월)참여연구자지급급여(4대보험본인부담포함)해당연(월)급여총액)",
            ),
            (
                "가동률= {연간가동시간} over {연간가동가능시간} = {2 TIMES 230} over {2000-8 TIMES 20} = {460} over {1840} =25%",
                "가동률=연간가동시간연간가동가능시간=2\u{d7}2302000-8\u{d7}20=4601840=25%",
            ),
            (
                "sqrt {{재해건수 TIMES 10 ^{6}} over {연`근로시간수}  TIMES  {근로손실일수 TIMES 10 ^{3}} over {연`근로시간수}}",
                "재해건수\u{d7}106연근로시간수\u{d7}근로손실일수\u{d7}103연근로시간수",
            ),
        ] {
            assert!(to_mathml(script).is_some(), "{script}");
            assert_eq!(shown_text(script).unwrap(), shown, "{script}");
        }
    }

    #[test]
    fn syntax_outside_the_subset_is_not_converted() {
        // A command this converter does not know, an unclosed or stray
        // brace, a fraction with nothing before it, a doubled script.
        for script in [
            "alpha + 1",
            "LEFT ( a RIGHT )",
            "{a over {b}",
            "a } b",
            "over {b}",
            "x _{1} _{2}",
            "{a} over",
        ] {
            assert_eq!(to_mathml(script), None, "{script}");
        }
    }

    #[test]
    fn text_is_escaped() {
        assert!(mathml("a<b").contains("<mo>&lt;</mo>"));
    }
}
