//! Structural regex parsing for Arr custom-format conditions.
//!
//! Arr stores .NET regexes. Rules evaluate them with Regorus, whose engine is
//! RE2-like. This module parses the shared Oniguruma-style surface syntax into
//! an AST so the translator can refuse constructs the target engine cannot
//! express, lower lookarounds into position relations, and regenerate the
//! plain fragments between them. Error sentences are part of the operator
//! diagnostics and are kept stable.

use super::js::is_js_whitespace;
use std::collections::HashMap;

type ParseResult<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AssertionKind {
    LineEnd,
    LineStart,
    SearchStart,
    StringEnd,
    StringEndNewline,
    StringStart,
    TextSegmentBoundary,
    WordBoundary,
}

impl AssertionKind {
    fn name(self) -> &'static str {
        match self {
            Self::LineEnd => "line_end",
            Self::LineStart => "line_start",
            Self::SearchStart => "search_start",
            Self::StringEnd => "string_end",
            Self::StringEndNewline => "string_end_newline",
            Self::StringStart => "string_start",
            Self::TextSegmentBoundary => "text_segment_boundary",
            Self::WordBoundary => "word_boundary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SetKind {
    Any,
    Digit,
    Dot,
    Hex,
    Newline,
    Posix,
    Property,
    Space,
    TextSegment,
    Word,
}

impl SetKind {
    fn name(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Digit => "digit",
            Self::Dot => "dot",
            Self::Hex => "hex",
            Self::Newline => "newline",
            Self::Posix => "posix",
            Self::Property => "property",
            Self::Space => "space",
            Self::TextSegment => "text_segment",
            Self::Word => "word",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuantifierKind {
    Greedy,
    Lazy,
    Possessive,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct FlagSwitches {
    ignore_case: bool,
    dot_all: bool,
    extended: bool,
}

impl FlagSwitches {
    fn parse(flags: &str) -> Option<Self> {
        let switches = Self {
            ignore_case: flags.contains('i'),
            dot_all: flags.contains('m'),
            extended: flags.contains('x'),
        };
        (switches != Self::default()).then_some(switches)
    }

    fn render(self) -> String {
        let mut out = String::new();
        if self.ignore_case {
            out.push('i');
        }
        if self.dot_all {
            out.push('m');
        }
        if self.extended {
            out.push('x');
        }
        out
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct FlagModifiers {
    enable: Option<FlagSwitches>,
    disable: Option<FlagSwitches>,
}

impl FlagModifiers {
    fn render(self) -> String {
        let enable = self.enable.map(FlagSwitches::render).unwrap_or_default();
        let disable = self.disable.map(FlagSwitches::render).unwrap_or_default();
        if disable.is_empty() {
            enable
        } else {
            format!("{enable}-{disable}")
        }
    }
}

/// Oniguruma AST nodes. Group-like bodies hold `Alternative` nodes.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Node {
    AbsenceFunction(Vec<Node>),
    Alternative(Vec<Node>),
    Assertion {
        kind: AssertionKind,
        negate: bool,
    },
    Backreference,
    CapturingGroup {
        name: Option<String>,
        body: Vec<Node>,
    },
    Character(u32),
    CharacterClass {
        intersection: bool,
        negate: bool,
        body: Vec<Node>,
    },
    CharacterClassRange(u32, u32),
    CharacterSet {
        kind: SetKind,
        negate: bool,
        value: Option<String>,
    },
    Directive(Option<FlagModifiers>),
    Group {
        atomic: bool,
        flags: Option<FlagModifiers>,
        body: Vec<Node>,
    },
    Lookaround {
        behind: bool,
        negate: bool,
        body: Vec<Node>,
    },
    NamedCallout {
        kind: String,
        tag: Option<String>,
        args: Option<Vec<String>>,
    },
    Quantifier {
        kind: QuantifierKind,
        min: u32,
        max: Option<u32>,
        body: Box<Node>,
    },
    Subroutine,
}

impl Node {
    /// Child nodes in the order a JavaScript property walk visits them.
    pub(super) fn children(&self) -> &[Node] {
        match self {
            Node::AbsenceFunction(body)
            | Node::Alternative(body)
            | Node::CapturingGroup { body, .. }
            | Node::CharacterClass { body, .. }
            | Node::Group { body, .. }
            | Node::Lookaround { body, .. } => body,
            Node::Quantifier { body, .. } => std::slice::from_ref(body.as_ref()),
            _ => &[],
        }
    }

    /// Whether the node keeps its children in an array body.
    pub(super) fn has_array_body(&self) -> bool {
        matches!(
            self,
            Node::AbsenceFunction(_)
                | Node::Alternative(_)
                | Node::CapturingGroup { .. }
                | Node::CharacterClass { .. }
                | Node::Group { .. }
                | Node::Lookaround { .. }
        )
    }

    fn is_quantifiable(&self) -> bool {
        matches!(
            self,
            Node::AbsenceFunction(_)
                | Node::Backreference
                | Node::CapturingGroup { .. }
                | Node::Character(_)
                | Node::CharacterClass { .. }
                | Node::CharacterSet { .. }
                | Node::Group { .. }
                | Node::Quantifier { .. }
                | Node::Subroutine
        )
    }
}

/// A parsed pattern: its top-level alternatives.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Regex {
    pub(super) body: Vec<Node>,
}

// ── Tokenizer ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupKind {
    AbsenceRepeater,
    Atomic,
    Capturing,
    Group,
    Lookahead,
    Lookbehind,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Alternator,
    Assertion(String),
    Backreference(String),
    Character {
        value: u32,
        literal_bracket: bool,
    },
    ClassClose,
    ClassHyphen,
    ClassIntersector,
    ClassOpen {
        negate: bool,
    },
    CharacterSet {
        kind: SetKind,
        negate: bool,
        value: Option<String>,
    },
    Directive(Option<FlagModifiers>),
    EscapedNumber {
        in_class: bool,
        raw: String,
    },
    GroupClose,
    GroupOpen(GroupOpen),
    NamedCallout {
        kind: String,
        tag: Option<String>,
        args: Option<Vec<String>>,
    },
    Quantifier {
        kind: QuantifierKind,
        min: u32,
        max: Option<u32>,
    },
    Subroutine(String),
}

#[derive(Debug, Clone, PartialEq)]
struct GroupOpen {
    kind: GroupKind,
    flags: Option<FlagModifiers>,
    name: Option<String>,
    number: u32,
    negate: bool,
    bare_paren: bool,
}

impl GroupOpen {
    fn new(kind: GroupKind) -> Self {
        Self {
            kind,
            flags: None,
            name: None,
            number: 0,
            negate: false,
            bare_paren: false,
        }
    }
}

fn character(value: u32) -> Token {
    Token::Character {
        value,
        literal_bracket: false,
    }
}

fn set(kind: SetKind, negate: bool) -> Token {
    Token::CharacterSet {
        kind,
        negate,
        value: None,
    }
}

fn is_ahex(c: Option<&char>) -> bool {
    c.is_some_and(char::is_ascii_hexdigit)
}

fn is_digit(c: Option<&char>) -> bool {
    c.is_some_and(char::is_ascii_digit)
}

fn is_ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Shared escape alternatives; `i` is the index just after the backslash.
fn match_shared_escape(s: &[char], i: usize) -> Option<usize> {
    let at = |index: usize| s.get(index).copied();
    match at(i)? {
        'c' => Some(if i + 1 < s.len() { i + 2 } else { i + 1 }),
        'C' => {
            if at(i + 1) == Some('-') {
                Some(if i + 2 < s.len() { i + 3 } else { i + 2 })
            } else {
                Some(i + 1)
            }
        }
        'p' | 'P' if at(i + 1) == Some('{') => {
            let mut j = i + 2;
            if at(j) == Some('^') {
                j += 1;
            }
            while matches!(at(j), Some('-' | ' ' | '_')) {
                j += 1;
            }
            if at(j).is_some_and(|c| c.is_ascii_alphabetic()) {
                j += 1;
                while at(j).is_some_and(|c| c == '-' || c == ' ' || is_ascii_word(c)) {
                    j += 1;
                }
                if at(j) == Some('}') {
                    return Some(j + 1);
                }
            }
            Some(i + 2)
        }
        'x' => {
            if at(i + 1).is_some_and(|c| matches!(c, '8' | '9' | 'A'..='F' | 'a'..='f'))
                && is_ahex(s.get(i + 2))
            {
                let mut j = i + 3;
                while at(j) == Some('\\')
                    && at(j + 1) == Some('x')
                    && at(j + 2).is_some_and(|c| matches!(c, '8' | '9' | 'A'..='F' | 'a'..='f'))
                    && is_ahex(s.get(j + 3))
                {
                    j += 4;
                }
                return Some(j);
            }
            if at(i + 1) == Some('{') {
                return Some(braced_end(s, i + 2));
            }
            let mut j = i + 1;
            while j < i + 3 && is_ahex(s.get(j)) {
                j += 1;
            }
            Some(j)
        }
        'u' => {
            if (1..=4).all(|offset| is_ahex(s.get(i + offset))) {
                Some(i + 5)
            } else {
                Some(i + 1)
            }
        }
        'o' if at(i + 1) == Some('{') => Some(braced_end(s, i + 2)),
        c if c.is_ascii_digit() => {
            let mut j = i + 1;
            while j < i + 3 && is_digit(s.get(j)) {
                j += 1;
            }
            Some(j)
        }
        _ => None,
    }
}

/// `[^}]*}?` starting at `j`.
fn braced_end(s: &[char], mut j: usize) -> usize {
    while j < s.len() && s[j] != '}' {
        j += 1;
    }
    if j < s.len() { j + 1 } else { j }
}

/// One `[?*+][?+]?|\{(?:\d+(?:,\d*)?|,\d+)\}\??` quantifier at `i`.
fn match_quantifier(s: &[char], i: usize) -> Option<usize> {
    let at = |index: usize| s.get(index).copied();
    match at(i)? {
        '?' | '*' | '+' => Some(if matches!(at(i + 1), Some('?' | '+')) {
            i + 2
        } else {
            i + 1
        }),
        '{' => {
            let mut j = i + 1;
            if is_digit(s.get(j)) {
                while is_digit(s.get(j)) {
                    j += 1;
                }
                if at(j) == Some(',') {
                    j += 1;
                    while is_digit(s.get(j)) {
                        j += 1;
                    }
                }
            } else if at(j) == Some(',') && is_digit(s.get(j + 1)) {
                j += 1;
                while is_digit(s.get(j)) {
                    j += 1;
                }
            } else {
                return None;
            }
            if at(j) != Some('}') {
                return None;
            }
            j += 1;
            if at(j) == Some('?') {
                j += 1;
            }
            Some(j)
        }
        _ => None,
    }
}

fn match_token(s: &[char], i: usize) -> usize {
    let at = |index: usize| s.get(index).copied();
    match s[i] {
        '\\' => {
            if i + 1 >= s.len() {
                return i + 1;
            }
            if let Some(end) = match_shared_escape(s, i + 1) {
                return end;
            }
            if matches!(at(i + 1), Some('g' | 'k')) {
                for (open, close) in [('<', '>'), ('\'', '\'')] {
                    if at(i + 2) == Some(open) {
                        let mut j = i + 3;
                        while j < s.len() && s[j] != close {
                            j += 1;
                        }
                        return if j < s.len() { j + 1 } else { j };
                    }
                }
            }
            i + 2
        }
        '(' => {
            if at(i + 1) == Some('?') {
                let k = i + 2;
                match at(k) {
                    Some(':' | '=' | '!' | '>' | '(' | '{') => return k + 1,
                    Some('<') if matches!(at(k + 1), Some('=' | '!')) => return k + 2,
                    _ => {}
                }
                for (open, close) in [('<', '>'), ('\'', '\'')] {
                    if at(k) == Some(open) {
                        if let Some(offset) = s[k + 1..].iter().position(|c| *c == close) {
                            return k + 1 + offset + 1;
                        }
                    }
                }
                if at(k) == Some('~') {
                    return if at(k + 1) == Some('|') { k + 2 } else { k + 1 };
                }
                if at(k) == Some('#') {
                    let mut j = k + 1;
                    while let Some(c) = at(j) {
                        if c == ')' {
                            break;
                        }
                        if c == '\\' {
                            j += if j + 1 < s.len() { 2 } else { 1 };
                        } else {
                            j += 1;
                        }
                    }
                    return j;
                }
                let mut j = k;
                while at(j).is_some_and(|c| c != ':' && c != ')') {
                    j += 1;
                }
                if j < s.len() {
                    return j + 1;
                }
                return k;
            }
            if at(i + 1) == Some('*') {
                let mut j = i + 2;
                while j < s.len() && s[j] != ')' {
                    j += 1;
                }
                return if j < s.len() { j + 1 } else { j };
            }
            i + 1
        }
        '[' => {
            if at(i + 1) == Some('^') {
                i + 2
            } else {
                i + 1
            }
        }
        _ => {
            let mut j = i;
            while let Some(end) = match_quantifier(s, j) {
                j = end;
            }
            if j > i { j } else { i + 1 }
        }
    }
}

fn match_class_token(s: &[char], i: usize) -> usize {
    let at = |index: usize| s.get(index).copied();
    match s[i] {
        '\\' => {
            if i + 1 >= s.len() {
                return i + 1;
            }
            match_shared_escape(s, i + 1).unwrap_or(i + 2)
        }
        '[' => {
            if at(i + 1) == Some(':') {
                let k = i + 2;
                let mut p = k;
                if at(p) == Some('^') {
                    p += 1;
                }
                if at(p).is_some_and(char::is_alphabetic) {
                    while at(p).is_some_and(char::is_alphabetic) {
                        p += 1;
                    }
                    if at(p) == Some(':') && at(p + 1) == Some(']') {
                        return p + 2;
                    }
                }
                if at(k) == Some('^') && at(k + 1) == Some(':') && at(k + 2) == Some(']') {
                    return k + 3;
                }
            }
            if at(i + 1) == Some('^') { i + 2 } else { i + 1 }
        }
        '&' if at(i + 1) == Some('&') => i + 2,
        _ => i + 1,
    }
}

struct Tokenizer<'a> {
    s: &'a [char],
    x_stack: Vec<bool>,
    open_groups: i64,
}

fn text_of(chars: &[char]) -> String {
    chars.iter().collect()
}

fn tokenize(pattern: &str) -> ParseResult<Vec<Token>> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut tokenizer = Tokenizer {
        s: &chars,
        x_stack: vec![false],
        open_groups: 0,
    };
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let end = match_token(&chars, i);
        i = tokenizer.token(i, end, &mut tokens)?;
    }

    let mut named_captures = 0;
    let mut potential = Vec::new();
    for (index, token) in tokens.iter_mut().enumerate() {
        if let Token::GroupOpen(open) = token {
            if open.kind == GroupKind::Capturing {
                named_captures += 1;
                open.number = named_captures;
            } else if open.bare_paren {
                potential.push(index);
            }
        }
    }
    if named_captures == 0 {
        for (position, index) in potential.iter().enumerate() {
            if let Token::GroupOpen(open) = &mut tokens[*index] {
                open.kind = GroupKind::Capturing;
                open.number = position as u32 + 1;
            }
        }
    }
    let captures = if named_captures > 0 {
        named_captures as usize
    } else {
        potential.len()
    };
    let mut out = Vec::with_capacity(tokens.len());
    for token in tokens {
        match token {
            Token::EscapedNumber { in_class, raw } => {
                out.extend(split_escaped_number(in_class, &raw, captures)?);
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

impl Tokenizer<'_> {
    fn current_x(&self) -> bool {
        self.x_stack.last().copied().unwrap_or(false)
    }

    /// Handles the token `s[start..end]`, returning the next scan index.
    fn token(&mut self, start: usize, end: usize, tokens: &mut Vec<Token>) -> ParseResult<usize> {
        let s = self.s;
        let m = &s[start..end];
        let raw = text_of(m);
        if raw == "[" || raw == "[^" {
            return self.class_tokens(raw == "[^", end, tokens);
        }
        let m1 = m.get(1).copied();
        if m[0] == '\\' {
            if m1.is_some_and(|c| "AbBGyYzZ".contains(c)) {
                tokens.push(Token::Assertion(raw));
                return Ok(end);
            }
            for (letter, make) in [
                ('g', Token::Subroutine as fn(String) -> Token),
                ('k', Token::Backreference as fn(String) -> Token),
            ] {
                if m1 == Some(letter) && matches!(m.get(2), Some('<' | '\'')) {
                    if !valid_group_reference(m) {
                        return Err(format!("Invalid group name \"{raw}\""));
                    }
                    tokens.push(make(raw));
                    return Ok(end);
                }
            }
            match m1 {
                Some('K') => {
                    tokens.push(Token::Directive(None));
                    return Ok(end);
                }
                Some(c @ ('N' | 'R')) => {
                    tokens.push(set(SetKind::Newline, c == 'N'));
                    return Ok(end);
                }
                Some('O') => {
                    tokens.push(set(SetKind::Any, false));
                    return Ok(end);
                }
                Some('X') => {
                    tokens.push(set(SetKind::TextSegment, false));
                    return Ok(end);
                }
                _ => {}
            }
            tokens.extend(shared_escape(m, false)?);
            return Ok(end);
        }
        if m[0] == '(' {
            if m1 == Some('*') {
                tokens.push(named_callout(m)?);
                return Ok(end);
            }
            if raw == "(?{" {
                return Err(format!("Unsupported callout \"{raw}\""));
            }
            if raw.starts_with("(?#") {
                if s.get(end) != Some(&')') {
                    return Err("Unclosed comment group \"(?#\"".to_string());
                }
                return Ok(end + 1);
            }
            if is_flag_modifier(m) {
                tokens.push(self.flag_modifier(&raw)?);
                return Ok(end);
            }
            let x = self.current_x();
            self.x_stack.push(x);
            self.open_groups += 1;
            let open = match raw.as_str() {
                "(" => GroupOpen {
                    bare_paren: true,
                    ..GroupOpen::new(GroupKind::Group)
                },
                "(?:" => GroupOpen::new(GroupKind::Group),
                "(?>" => GroupOpen::new(GroupKind::Atomic),
                "(?=" | "(?!" | "(?<=" | "(?<!" => GroupOpen {
                    negate: raw.ends_with('!'),
                    ..GroupOpen::new(if m[2] == '<' {
                        GroupKind::Lookbehind
                    } else {
                        GroupKind::Lookahead
                    })
                },
                _ if (raw.starts_with("(?<") && raw.ends_with('>'))
                    || (raw.starts_with("(?'") && raw.ends_with('\'') && m.len() > 3) =>
                {
                    GroupOpen {
                        name: Some(text_of(&m[3..m.len() - 1])),
                        ..GroupOpen::new(GroupKind::Capturing)
                    }
                }
                "(?~|" => return Err(format!("Unsupported absence function kind \"{raw}\"")),
                _ if raw.starts_with("(?~") => GroupOpen::new(GroupKind::AbsenceRepeater),
                "(?(" => return Err(format!("Unsupported conditional \"{raw}\"")),
                _ => return Err(format!("Invalid or unsupported group option \"{raw}\"")),
            };
            tokens.push(Token::GroupOpen(open));
            return Ok(end);
        }
        if raw == ")" {
            self.x_stack.pop();
            self.open_groups -= 1;
            if self.open_groups < 0 {
                return Err("Unmatched \")\"".to_string());
            }
            tokens.push(Token::GroupClose);
            return Ok(end);
        }
        if self.current_x() {
            if raw == "#" {
                let newline = s[end..].iter().position(|c| *c == '\n');
                return Ok(newline.map_or(s.len(), |offset| end + offset));
            }
            if m.len() == 1 && is_js_whitespace(m[0]) {
                let mut j = end;
                while j < s.len() && is_js_whitespace(s[j]) {
                    j += 1;
                }
                return Ok(j);
            }
        }
        match raw.as_str() {
            "." => tokens.push(set(SetKind::Dot, false)),
            "^" | "$" => tokens.push(Token::Assertion(raw)),
            "|" => tokens.push(Token::Alternator),
            _ if match_quantifier(m, 0).is_some() => tokens.extend(split_quantifiers(m)?),
            _ => tokens.push(character(m[0] as u32)),
        }
        Ok(end)
    }

    fn flag_modifier(&mut self, raw: &str) -> ParseResult<Token> {
        let inner = &raw[2..raw.len() - 1];
        let (on, off) = match inner.split_once('-') {
            Some((on, off)) => (on, off),
            None => (inner, ""),
        };
        let x_on = (self.current_x() || on.contains('x')) && !off.contains('x');
        let modifiers = FlagModifiers {
            enable: FlagSwitches::parse(on),
            disable: FlagSwitches::parse(off),
        };
        if raw.ends_with(')') {
            if let Some(last) = self.x_stack.last_mut() {
                *last = x_on;
            }
            return Ok(Token::Directive(Some(modifiers)));
        }
        self.x_stack.push(x_on);
        self.open_groups += 1;
        let has_flags = modifiers.enable.is_some() || modifiers.disable.is_some();
        Ok(Token::GroupOpen(GroupOpen {
            flags: has_flags.then_some(modifiers),
            ..GroupOpen::new(GroupKind::Group)
        }))
    }

    fn class_tokens(
        &self,
        negate: bool,
        mut i: usize,
        tokens: &mut Vec<Token>,
    ) -> ParseResult<usize> {
        let s = self.s;
        tokens.push(Token::ClassOpen { negate });
        let mut open = 1;
        while i < s.len() {
            let end = match_class_token(s, i);
            let m = &s[i..end];
            i = end;
            if m[0] == '[' && m.get(1) != Some(&':') {
                open += 1;
                tokens.push(Token::ClassOpen {
                    negate: m.get(1) == Some(&'^'),
                });
            } else if m == [']'] {
                if matches!(tokens.last(), Some(Token::ClassOpen { .. })) {
                    tokens.push(Token::Character {
                        value: 93,
                        literal_bracket: true,
                    });
                } else {
                    open -= 1;
                    tokens.push(Token::ClassClose);
                    if open == 0 {
                        break;
                    }
                }
            } else {
                tokens.extend(class_member(m)?);
            }
        }
        Ok(i)
    }
}

fn valid_group_reference(m: &[char]) -> bool {
    let (open, close) = if m[2] == '<' {
        ('<', '>')
    } else {
        ('\'', '\'')
    };
    let inner = &m[3..];
    inner.len() >= 2
        && inner.last() == Some(&close)
        && inner[..inner.len() - 1].iter().all(|c| *c != close)
        && open != '\0'
}

fn is_flag_modifier(m: &[char]) -> bool {
    m.len() >= 4
        && m[0] == '('
        && m[1] == '?'
        && matches!(m[m.len() - 1], ':' | ')')
        && m[2..m.len() - 1]
            .iter()
            .all(|c| matches!(c, '-' | 'i' | 'm' | 'x'))
}

fn class_member(m: &[char]) -> ParseResult<Vec<Token>> {
    if m[0] == '\\' {
        return shared_escape(m, true);
    }
    if m[0] == '[' {
        let raw = text_of(m);
        let inner = &m[2..m.len() - 2];
        let (negate, name) = match inner.first() {
            Some('^') => (true, &inner[1..]),
            _ => (false, inner),
        };
        let name = text_of(name);
        if name.is_empty()
            || !name.chars().all(|c| c.is_ascii_lowercase())
            || !POSIX_CLASS_NAMES.contains(&name.as_str())
        {
            return Err(format!("Invalid POSIX class \"{raw}\""));
        }
        return Ok(vec![Token::CharacterSet {
            kind: SetKind::Posix,
            negate,
            value: Some(name),
        }]);
    }
    if m == ['-'] {
        return Ok(vec![Token::ClassHyphen]);
    }
    if m == ['&', '&'] {
        return Ok(vec![Token::ClassIntersector]);
    }
    Ok(vec![character(m[0] as u32)])
}

const POSIX_CLASS_NAMES: &[&str] = &[
    "alnum", "alpha", "ascii", "blank", "cntrl", "digit", "graph", "lower", "print", "punct",
    "space", "upper", "word", "xdigit",
];

fn shared_escape(m: &[char], in_class: bool) -> ParseResult<Vec<Token>> {
    let raw = text_of(m);
    let char1 = m.get(1).copied();
    match char1 {
        Some('c' | 'C') => {
            let letter = if char1 == Some('c') {
                m.get(2)
            } else {
                m.get(3)
            };
            return match letter {
                Some(c) if c.is_ascii_alphabetic() => {
                    Ok(vec![character(c.to_ascii_uppercase() as u32 - 64)])
                }
                _ => Err(format!("Unsupported control character \"{raw}\"")),
            };
        }
        Some(c) if "dDhHsSwW".contains(c) => {
            let lower = c.to_ascii_lowercase();
            let kind = match lower {
                'd' => SetKind::Digit,
                'h' => SetKind::Hex,
                's' => SetKind::Space,
                _ => SetKind::Word,
            };
            return Ok(vec![set(kind, c != lower)]);
        }
        _ => {}
    }
    if raw.starts_with("\\o{") {
        return Err(format!(
            "Incomplete, invalid, or unsupported octal code point \"{raw}\""
        ));
    }
    if matches!(char1, Some('p' | 'P')) && m.get(2) == Some(&'{') {
        if m.len() == 3 {
            return Err(format!("Incomplete or invalid Unicode property \"{raw}\""));
        }
        let mut rest = &m[3..];
        let neg = rest.first() == Some(&'^');
        if neg {
            rest = &rest[1..];
        }
        let value: String = rest.iter().take_while(|c| **c != '}').collect();
        let negate = (char1 == Some('P') && !neg) || (char1 == Some('p') && neg);
        return Ok(vec![property(value, negate)]);
    }
    if char1 == Some('x')
        && m.get(2)
            .is_some_and(|c| matches!(c, '8' | '9' | 'A'..='F' | 'a'..='f'))
        && is_ahex(m.get(3))
    {
        let bytes: Option<Vec<u8>> = raw
            .split("\\x")
            .skip(1)
            .map(|hex| u8::from_str_radix(hex, 16).ok())
            .collect();
        let decoded = bytes.and_then(|bytes| String::from_utf8(bytes).ok());
        return match decoded {
            Some(decoded) => Ok(decoded.chars().map(|c| character(c as u32)).collect()),
            None => Err(format!(
                "Multibyte code \"{raw}\" incomplete or invalid in Oniguruma"
            )),
        };
    }
    if matches!(char1, Some('u' | 'x')) {
        return Ok(vec![character(hex_char_code(m)?)]);
    }
    if let Some(value) = char1.and_then(|c| match c {
        'a' => Some(7),
        'b' => Some(8),
        'e' => Some(27),
        'f' => Some(12),
        'n' => Some(10),
        'r' => Some(13),
        't' => Some(9),
        'v' => Some(11),
        _ => None,
    }) {
        return Ok(vec![character(value)]);
    }
    if char1.is_some_and(|c| c.is_ascii_digit()) {
        return Ok(vec![Token::EscapedNumber { in_class, raw }]);
    }
    if raw == "\\" {
        return Err("Incomplete escape \"\\\"".to_string());
    }
    if char1 == Some('M') {
        return Err(format!("Unsupported meta \"{raw}\""));
    }
    if m.len() == 2 {
        return Ok(vec![character(m[1] as u32)]);
    }
    Err(format!("Unexpected escape \"{raw}\""))
}

fn property(value: String, negate: bool) -> Token {
    let normalized = slug(&value);
    if POSIX_CLASS_NAMES.contains(&normalized.as_str()) {
        Token::CharacterSet {
            kind: SetKind::Posix,
            negate,
            value: Some(normalized),
        }
    } else {
        Token::CharacterSet {
            kind: SetKind::Property,
            negate,
            value: Some(value),
        }
    }
}

fn slug(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '-' | ' ' | '_'))
        .collect::<String>()
        .to_lowercase()
}

fn hex_char_code(m: &[char]) -> ParseResult<u32> {
    let raw = text_of(m);
    let rest = &m[2..];
    let valid = if m[1] == 'u' {
        rest.len() >= 4 && rest[..4].iter().all(char::is_ascii_hexdigit)
    } else if rest.first().is_some_and(char::is_ascii_hexdigit) {
        true
    } else if rest.first() == Some(&'{') {
        let digits = rest[1..]
            .iter()
            .take_while(|c| c.is_ascii_hexdigit())
            .count();
        (1..=8).contains(&digits) && rest.get(1 + digits) == Some(&'}')
    } else {
        false
    };
    if !valid {
        return Err(format!("Incomplete or invalid escape \"{raw}\""));
    }
    let hex: String = if rest.first() == Some(&'{') {
        rest[1..]
            .iter()
            .skip_while(|c| is_js_whitespace(**c))
            .take_while(|c| c.is_ascii_hexdigit())
            .collect()
    } else {
        rest.iter().collect()
    };
    u32::from_str_radix(&hex, 16).map_err(|_| format!("Incomplete or invalid escape \"{raw}\""))
}

fn split_escaped_number(in_class: bool, raw: &str, captures: usize) -> ParseResult<Vec<Token>> {
    let value = &raw[1..];
    let first = value.as_bytes()[0];
    let numeric = value.parse::<usize>().unwrap_or(usize::MAX);
    if !in_class && ((value != "0" && value.len() == 1) || (first != b'0' && numeric <= captures)) {
        return Ok(vec![Token::Backreference(raw.to_string())]);
    }
    let mut parts: Vec<&str> = Vec::new();
    let octal_run = value
        .bytes()
        .take_while(|b| (b'0'..=b'7').contains(b))
        .count();
    let mut rest = value;
    if octal_run > 0 {
        parts.push(&value[..octal_run]);
        rest = &value[octal_run..];
    }
    for index in 0..rest.len() {
        parts.push(&rest[index..index + 1]);
    }
    let mut tokens = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let code = if index == 0 && *part != "8" && *part != "9" {
            let code = u32::from_str_radix(part, 8).unwrap_or(u32::MAX);
            if code > 0o177 {
                return Err(format!(
                    "Octal encoded byte above 177 unsupported \"{raw}\""
                ));
            }
            code
        } else {
            part.as_bytes()[0] as u32
        };
        tokens.push(character(code));
    }
    Ok(tokens)
}

fn split_quantifiers(m: &[char]) -> ParseResult<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(end) = match_quantifier(m, i) {
        let q = &m[i..end];
        if q[0] == '{' && q.last() == Some(&'?') {
            let body = &q[1..q.len() - 2];
            let text = text_of(body);
            if let Some((min, max)) = text.split_once(',') {
                if !min.is_empty() && !max.is_empty() {
                    let (min, max) = (big(min), big(max));
                    if min > max {
                        tokens.push(quantifier(&q[..q.len() - 1])?);
                        i = end - 1;
                        continue;
                    }
                }
            }
        }
        tokens.push(quantifier(q)?);
        i = end;
    }
    Ok(tokens)
}

/// A decimal string as a JavaScript number would compare it.
fn big(digits: &str) -> f64 {
    digits.parse::<f64>().unwrap_or(f64::INFINITY)
}

const QUANTIFIER_LIMIT: f64 = 100_000.0;

fn quantifier(raw: &[char]) -> ParseResult<Token> {
    if raw[0] == '{' {
        let inner: String = raw[1..].iter().take_while(|c| **c != '}').collect();
        let (min_text, max_text) = match inner.split_once(',') {
            Some((min, max)) => (min.to_string(), Some(max.to_string())),
            None => (inner.clone(), None),
        };
        let min_value = if min_text.is_empty() {
            0.0
        } else {
            big(&min_text)
        };
        let max_value = max_text.as_deref().filter(|max| !max.is_empty()).map(big);
        if min_value > QUANTIFIER_LIMIT || max_value.is_some_and(|max| max > QUANTIFIER_LIMIT) {
            return Err("Quantifier value unsupported in Oniguruma".to_string());
        }
        let mut min = Some(min_value as u32);
        let mut max = match &max_text {
            None => Some(min_value as u32),
            Some(max) if max.is_empty() => None,
            Some(_) => max_value.map(|max| max as u32),
        };
        let mut kind = None;
        if max.is_some_and(|max| min.is_some_and(|min| min > max)) {
            kind = Some(QuantifierKind::Possessive);
            std::mem::swap(&mut min, &mut max);
        }
        if raw.last() == Some(&'?') {
            if kind == Some(QuantifierKind::Possessive) {
                return Err(
                    "Unsupported possessive interval quantifier chain with \"?\"".to_string(),
                );
            }
            kind = Some(QuantifierKind::Lazy);
        }
        return Ok(Token::Quantifier {
            kind: kind.unwrap_or(QuantifierKind::Greedy),
            min: min.unwrap_or(0),
            max,
        });
    }
    let min = u32::from(raw[0] == '+');
    let max = (raw[0] == '?').then_some(1);
    let kind = match raw.get(1) {
        Some('+') => QuantifierKind::Possessive,
        Some('?') => QuantifierKind::Lazy,
        _ => QuantifierKind::Greedy,
    };
    Ok(Token::Quantifier { kind, min, max })
}

const CALLOUT_NAMES: &[&str] = &[
    "COUNT",
    "CMP",
    "ERROR",
    "FAIL",
    "MAX",
    "MISMATCH",
    "SKIP",
    "TOTAL_COUNT",
];

struct CalloutParts {
    name: Option<String>,
    tag: Option<String>,
    args: Option<String>,
}

fn callout_at(m: &[char], start: usize) -> Option<CalloutParts> {
    let at = |index: usize| m.get(index).copied();
    if at(start) != Some('(') || at(start + 1) != Some('*') {
        return None;
    }
    let identifier = |mut j: usize| -> usize {
        if at(j).is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
            j += 1;
            while at(j).is_some_and(is_ascii_word) {
                j += 1;
            }
        }
        j
    };
    let mut j = start + 2;
    let name_end = identifier(j);
    let name = (name_end > j).then(|| text_of(&m[j..name_end]));
    j = name_end;
    let mut tag = None;
    if at(j) == Some('[') {
        let tag_end = identifier(j + 1);
        if at(tag_end) != Some(']') {
            return None;
        }
        tag = Some(text_of(&m[j + 1..tag_end]));
        j = tag_end + 1;
    }
    let mut args = None;
    if at(j) == Some('{') {
        let close = m[j + 1..].iter().position(|c| *c == '}')? + j + 1;
        args = Some(text_of(&m[j + 1..close]));
        j = close + 1;
    }
    (at(j) == Some(')')).then_some(CalloutParts { name, tag, args })
}

#[derive(Clone)]
enum CalloutArg {
    Number(f64),
    Text(String),
}

impl CalloutArg {
    fn display(&self) -> String {
        match self {
            CalloutArg::Number(value) => super::js::js_number_to_string(*value),
            CalloutArg::Text(text) => text.clone(),
        }
    }

    fn is_identifier(&self) -> bool {
        match self {
            CalloutArg::Number(_) => false,
            CalloutArg::Text(text) => {
                let mut chars = text.chars();
                chars
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && chars.all(is_ascii_word)
            }
        }
    }

    fn is_text(&self) -> bool {
        matches!(self, CalloutArg::Text(_))
    }

    fn is_one_of(&self, choices: &[&str]) -> bool {
        matches!(self, CalloutArg::Text(text) if choices.contains(&text.as_str()))
    }
}

fn named_callout(m: &[char]) -> ParseResult<Token> {
    let raw = text_of(m);
    let parts = (0..m.len())
        .find_map(|start| callout_at(m, start))
        .ok_or_else(|| format!("Incomplete or invalid named callout \"{raw}\""))?;
    let Some(name) = parts.name else {
        return Err(format!("Invalid named callout \"{raw}\""));
    };
    if parts.tag.as_deref() == Some("") {
        return Err(format!(
            "Named callout tag with empty value not allowed \"{raw}\""
        ));
    }
    let args: Vec<CalloutArg> = parts
        .args
        .as_deref()
        .map(|args| {
            args.split(',')
                .filter(|arg| !arg.is_empty())
                .map(|arg| {
                    let digits = arg.strip_prefix(['+', '-']).unwrap_or(arg);
                    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                        CalloutArg::Number(arg.parse::<f64>().unwrap_or(f64::NAN))
                    } else {
                        CalloutArg::Text(arg.to_string())
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let list = args
        .iter()
        .map(CalloutArg::display)
        .collect::<Vec<_>>()
        .join(",");
    let kind = if CALLOUT_NAMES.contains(&name.as_str()) {
        name.to_lowercase()
    } else {
        "custom".to_string()
    };
    let arg = |index: usize| args.get(index);
    let text_not_identifier =
        |index: usize| arg(index).is_some_and(|a| a.is_text() && !a.is_identifier());
    match kind.as_str() {
        "fail" | "mismatch" | "skip" if !args.is_empty() => {
            return Err(format!("Named callout arguments not allowed \"{list}\""));
        }
        "error" => {
            if args.len() > 1 {
                return Err(format!("Named callout allows only one argument \"{list}\""));
            }
            if let Some(first @ CalloutArg::Text(_)) = arg(0) {
                return Err(format!(
                    "Named callout argument must be a number \"{}\"",
                    first.display()
                ));
            }
        }
        "max" => {
            if args.is_empty() || args.len() > 2 {
                return Err(format!(
                    "Named callout must have one or two arguments \"{list}\""
                ));
            }
            if text_not_identifier(0) {
                return Err(format!(
                    "Named callout argument one must be a tag or number \"{}\"",
                    args[0].display()
                ));
            }
            if args.len() == 2 && !args[1].is_one_of(&["<", ">", "X"]) {
                return Err(format!(
                    "Named callout optional argument two must be '<', '>', or 'X' \"{}\"",
                    args[1].display()
                ));
            }
        }
        "count" | "total_count" => {
            if args.len() > 1 {
                return Err(format!("Named callout allows only one argument \"{list}\""));
            }
            if args.len() == 1 && !args[0].is_one_of(&["<", ">", "X"]) {
                return Err(format!(
                    "Named callout optional argument must be '<', '>', or 'X' \"{}\"",
                    args[0].display()
                ));
            }
        }
        "cmp" => {
            if args.len() != 3 {
                return Err(format!(
                    "Named callout must have three arguments \"{list}\""
                ));
            }
            if text_not_identifier(0) {
                return Err(format!(
                    "Named callout argument one must be a tag or number \"{}\"",
                    args[0].display()
                ));
            }
            if !args[1].is_one_of(&["==", "!=", "<=", ">=", "<", ">"]) {
                return Err(format!(
                    "Named callout argument two must be '==', '!=', '>', '<', '>=', or '<=' \"{}\"",
                    args[1].display()
                ));
            }
            if text_not_identifier(2) {
                return Err(format!(
                    "Named callout argument three must be a tag or number \"{}\"",
                    args[2].display()
                ));
            }
        }
        "custom" => return Err(format!("Undefined callout name \"{name}\"")),
        _ => {}
    }
    Ok(Token::NamedCallout {
        kind,
        tag: parts.tag,
        args: parts
            .args
            .map(|args| args.split(',').map(str::to_string).collect()),
    })
}

// ── Parser ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Default)]
struct State {
    checking_range_end: bool,
    in_absence: bool,
}

enum SubroutineRef {
    Number(u64),
    Name(String),
}

struct Parser {
    tokens: Vec<Token>,
    next: usize,
    capturing_groups: usize,
    named_groups: HashMap<String, usize>,
    has_numbered_ref: bool,
    subroutines: Vec<SubroutineRef>,
}

/// Parses a pattern without Oniguruma's lookbehind restrictions; .NET
/// accepts variable-width lookbehind, which the translator lowers itself.
pub(super) fn parse(pattern: &str) -> ParseResult<Regex> {
    let mut parser = Parser {
        tokens: tokenize(pattern)?,
        next: 0,
        capturing_groups: 0,
        named_groups: HashMap::new(),
        has_numbered_ref: false,
        subroutines: Vec::new(),
    };
    let mut alternatives = vec![Vec::new()];
    while parser.next < parser.tokens.len() {
        let top = alternatives.last_mut().expect("at least one alternative");
        let node = parser.walk(top, State::default())?;
        if matches!(node, Node::Alternative(_)) {
            alternatives.push(Vec::new());
        } else {
            alternatives
                .last_mut()
                .expect("at least one alternative")
                .push(node);
        }
    }
    if parser.has_numbered_ref && !parser.named_groups.is_empty() {
        return Err("Numbered backref/subroutine not allowed when using named capture".to_string());
    }
    for reference in &parser.subroutines {
        match reference {
            SubroutineRef::Number(number) => {
                if *number > parser.capturing_groups as u64 {
                    return Err("Subroutine uses a group number that's not defined".to_string());
                }
            }
            SubroutineRef::Name(name) => match parser.named_groups.get(name) {
                None => {
                    return Err(format!(
                        "Subroutine uses a group name that's not defined \"\\g<{name}>\""
                    ));
                }
                Some(count) if *count > 1 => {
                    return Err(format!(
                        "Subroutine uses a duplicate group name \"\\g<{name}>\""
                    ));
                }
                Some(_) => {}
            },
        }
    }
    Ok(Regex {
        body: alternatives.into_iter().map(Node::Alternative).collect(),
    })
}

fn create_character(value: u32, use_last_valid: bool) -> ParseResult<Node> {
    if value > 0x10FFFF {
        let hex = format!("{value:x}");
        if use_last_valid {
            return Ok(Node::Character(0x10FFFF));
        }
        if value > 0x13FFFF {
            return Err(format!("Invalid code point out of range \"\\x{{{hex}}}\""));
        }
        return Err(format!(
            "Invalid code point out of range in JS \"\\x{{{hex}}}\""
        ));
    }
    Ok(Node::Character(value))
}

fn token_type(token: &Token) -> &'static str {
    match token {
        Token::Alternator => "Alternator",
        Token::Assertion(_) => "Assertion",
        Token::Backreference(_) => "Backreference",
        Token::Character { .. } => "Character",
        Token::ClassClose => "CharacterClassClose",
        Token::ClassHyphen => "CharacterClassHyphen",
        Token::ClassIntersector => "CharacterClassIntersector",
        Token::ClassOpen { .. } => "CharacterClassOpen",
        Token::CharacterSet { .. } => "CharacterSet",
        Token::Directive(_) => "Directive",
        Token::EscapedNumber { .. } => "EscapedNumber",
        Token::GroupClose => "GroupClose",
        Token::GroupOpen(_) => "GroupOpen",
        Token::NamedCallout { .. } => "NamedCallout",
        Token::Quantifier { .. } => "Quantifier",
        Token::Subroutine(_) => "Subroutine",
    }
}

fn valid_group_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| {
        c.is_alphabetic()
            || matches!(
                c,
                '_' | '\u{203F}'
                    | '\u{2040}'
                    | '\u{2054}'
                    | '\u{FE33}'
                    | '\u{FE34}'
                    | '\u{FE4D}'
                    | '\u{FE4E}'
                    | '\u{FE4F}'
                    | '\u{FF3F}'
            )
    }) && chars.all(|c| c != ')')
}

/// `^(sign)0*([1-9]\d*)$` for numbered group references.
fn numbered_reference(reference: &str, signs: &[char]) -> Option<(Option<char>, u64)> {
    let (sign, digits) = match reference.chars().next() {
        Some(c) if signs.contains(&c) => (Some(c), &reference[1..]),
        _ => (None, reference),
    };
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((sign, digits.parse::<u64>().unwrap_or(u64::MAX)))
}

impl Parser {
    fn walk(&mut self, parent: &mut Vec<Node>, state: State) -> ParseResult<Node> {
        let token = self.tokens[self.next].clone();
        self.next += 1;
        match token {
            Token::Alternator => Ok(Node::Alternative(Vec::new())),
            Token::Assertion(kind) => Ok(parse_assertion(&kind)),
            Token::Backreference(raw) => self.backreference(&raw),
            Token::Character { value, .. } => create_character(value, state.checking_range_end),
            Token::ClassHyphen => self.class_hyphen(parent, state),
            Token::ClassOpen { negate } => self.class_open(negate, state),
            Token::CharacterSet {
                kind,
                negate,
                value,
            } => Ok(Node::CharacterSet {
                kind,
                negate,
                value,
            }),
            Token::Directive(flags) => Ok(Node::Directive(flags)),
            Token::GroupOpen(open) => self.group_open(open, state),
            Token::NamedCallout { kind, tag, args } => Ok(Node::NamedCallout { kind, tag, args }),
            Token::Quantifier { kind, min, max } => {
                let Some(last) = parent.last() else {
                    return Err("Quantifier requires a repeatable token".to_string());
                };
                if !last.is_quantifiable() {
                    return Err("Quantifier requires a repeatable token".to_string());
                }
                let body = parent.pop().expect("checked above");
                Ok(Node::Quantifier {
                    kind,
                    min,
                    max,
                    body: Box::new(body),
                })
            }
            Token::Subroutine(raw) => self.subroutine(&raw),
            other => Err(format!("Unexpected token type \"{}\"", token_type(&other))),
        }
    }

    fn backreference(&mut self, raw: &str) -> ParseResult<Node> {
        let has_wrapper = raw.starts_with("\\k<") || raw.starts_with("\\k'");
        let reference = if has_wrapper {
            &raw[3..raw.len() - 1]
        } else {
            &raw[1..]
        };
        let from_number = |parser: &mut Parser, number: u64| -> ParseResult<Node> {
            if number > parser.capturing_groups as u64 {
                return Err(format!(
                    "Not enough capturing groups defined to the left \"{raw}\""
                ));
            }
            parser.has_numbered_ref = true;
            Ok(Node::Backreference)
        };
        if has_wrapper {
            if let Some((_, number)) = numbered_reference(reference, &['-']) {
                return from_number(self, number);
            }
            if reference.contains(['-', '+']) {
                return Err(format!("Invalid backref name \"{raw}\""));
            }
            if !self.named_groups.contains_key(reference) {
                return Err(format!("Group name not defined to the left \"{raw}\""));
            }
            return Ok(Node::Backreference);
        }
        from_number(self, reference.parse::<u64>().unwrap_or(u64::MAX))
    }

    fn subroutine(&mut self, raw: &str) -> ParseResult<Node> {
        let reference = &raw[3..raw.len() - 1];
        if let Some((sign, number)) = numbered_reference(reference, &['-', '+']) {
            let left = self.capturing_groups as i128;
            self.has_numbered_ref = true;
            let resolved = match sign {
                None => number as i128,
                Some('+') => left + number as i128,
                _ => left + 1 - number as i128,
            };
            if resolved < 1 {
                return Err("Invalid subroutine number".to_string());
            }
            self.subroutines.push(SubroutineRef::Number(
                u64::try_from(resolved).unwrap_or(u64::MAX),
            ));
        } else if reference == "0" {
            self.subroutines.push(SubroutineRef::Number(0));
        } else {
            self.subroutines
                .push(SubroutineRef::Name(reference.to_string()));
        }
        Ok(Node::Subroutine)
    }

    fn class_hyphen(&mut self, parent: &mut Vec<Node>, state: State) -> ParseResult<Node> {
        let previous_ok = parent.last().is_some_and(|node| {
            !matches!(
                node,
                Node::CharacterClass { .. } | Node::CharacterClassRange(..)
            )
        });
        let next_ok = self.tokens.get(self.next).is_some_and(|token| {
            !matches!(
                token,
                Token::ClassOpen { .. } | Token::ClassClose | Token::ClassIntersector
            )
        });
        if !state.checking_range_end && previous_ok && next_ok {
            let next = self.walk(
                parent,
                State {
                    checking_range_end: true,
                    ..state
                },
            )?;
            if let (Some(Node::Character(min)), Node::Character(max)) = (parent.last(), &next) {
                let (min, max) = (*min, *max);
                parent.pop();
                if max < min {
                    return Err("Character class range out of order".to_string());
                }
                return Ok(Node::CharacterClassRange(min, max));
            }
            return Err("Invalid character class range".to_string());
        }
        Ok(Node::Character('-' as u32))
    }

    fn class_open(&mut self, negate: bool, state: State) -> ParseResult<Node> {
        let started_with_bracket = matches!(
            self.tokens.get(self.next),
            Some(Token::Character {
                value: 93,
                literal_bracket: true
            })
        );
        let unclosed = |suffix: bool| {
            format!(
                "Unclosed character class{}",
                if suffix { " (started with \"]\")" } else { "" }
            )
        };
        let mut intersections: Vec<Vec<Node>> = vec![Vec::new()];
        let mut token = self
            .tokens
            .get(self.next)
            .cloned()
            .ok_or_else(|| unclosed(false))?;
        while token != Token::ClassClose {
            if token == Token::ClassIntersector {
                intersections.push(Vec::new());
                self.next += 1;
            } else {
                let mut body = intersections.pop().expect("at least one intersection");
                let node = self.walk(&mut body, state);
                match node {
                    Ok(node) => {
                        body.push(node);
                        intersections.push(body);
                    }
                    Err(error) => return Err(error),
                }
            }
            token = self
                .tokens
                .get(self.next)
                .cloned()
                .ok_or_else(|| unclosed(started_with_bracket))?;
        }
        self.next += 1;
        if intersections.len() == 1 {
            return Ok(Node::CharacterClass {
                intersection: false,
                negate,
                body: intersections.pop().expect("one intersection"),
            });
        }
        Ok(Node::CharacterClass {
            intersection: true,
            negate,
            body: intersections
                .into_iter()
                .map(|mut body| {
                    if body.len() == 1 {
                        body.pop().expect("one member")
                    } else {
                        Node::CharacterClass {
                            intersection: false,
                            negate: false,
                            body,
                        }
                    }
                })
                .collect(),
        })
    }

    fn group_open(&mut self, open: GroupOpen, state: State) -> ParseResult<Node> {
        let is_absence = open.kind == GroupKind::AbsenceRepeater;
        if open.kind == GroupKind::Capturing {
            if let Some(name) = &open.name {
                if !valid_group_name(name) {
                    return Err(format!("Group name \"{name}\" invalid in Oniguruma"));
                }
            }
            self.capturing_groups += 1;
            if let Some(name) = open.name.as_ref().filter(|name| !name.is_empty()) {
                *self.named_groups.entry(name.clone()).or_default() += 1;
            }
        }
        if is_absence && state.in_absence {
            return Err("Nested absence function not supported by Oniguruma".to_string());
        }
        let child_state = State {
            in_absence: state.in_absence || is_absence,
            ..state
        };
        let mut alternatives: Vec<Vec<Node>> = vec![Vec::new()];
        loop {
            let token = self
                .tokens
                .get(self.next)
                .ok_or_else(|| "Unclosed group".to_string())?;
            match token {
                Token::GroupClose => break,
                Token::Alternator => {
                    alternatives.push(Vec::new());
                    self.next += 1;
                }
                _ => {
                    let mut alternative = alternatives.pop().expect("at least one alternative");
                    let child = self.walk(&mut alternative, child_state)?;
                    alternative.push(child);
                    alternatives.push(alternative);
                }
            }
        }
        self.next += 1;
        let body = alternatives.into_iter().map(Node::Alternative).collect();
        Ok(match open.kind {
            GroupKind::AbsenceRepeater => Node::AbsenceFunction(body),
            GroupKind::Atomic => Node::Group {
                atomic: true,
                flags: None,
                body,
            },
            GroupKind::Capturing => Node::CapturingGroup {
                name: open.name.filter(|name| !name.is_empty()),
                body,
            },
            GroupKind::Group => Node::Group {
                atomic: false,
                flags: open.flags,
                body,
            },
            GroupKind::Lookahead | GroupKind::Lookbehind => Node::Lookaround {
                behind: open.kind == GroupKind::Lookbehind,
                negate: open.negate,
                body,
            },
        })
    }
}

fn parse_assertion(kind: &str) -> Node {
    let (kind, negate) = match kind {
        "^" => (AssertionKind::LineStart, false),
        "$" => (AssertionKind::LineEnd, false),
        "\\A" => (AssertionKind::StringStart, false),
        "\\b" => (AssertionKind::WordBoundary, false),
        "\\B" => (AssertionKind::WordBoundary, true),
        "\\G" => (AssertionKind::SearchStart, false),
        "\\y" => (AssertionKind::TextSegmentBoundary, false),
        "\\Y" => (AssertionKind::TextSegmentBoundary, true),
        "\\z" => (AssertionKind::StringEnd, false),
        _ => (AssertionKind::StringEndNewline, false),
    };
    Node::Assertion { kind, negate }
}

// ── Capability gate ──────────────────────────────────────────────────────

/// Refuses constructs Rego's regex engine cannot express, in the order a
/// pre-order walk meets them.
pub(super) fn check_capabilities(regex: &Regex) -> Result<(), String> {
    regex.body.iter().try_for_each(check_node)
}

fn check_flags(flags: Option<FlagModifiers>) -> Result<(), String> {
    let Some(flags) = flags else { return Ok(()) };
    if flags
        .enable
        .is_some_and(|enable| enable.dot_all || enable.extended)
    {
        return Err("Unsupported inline regex flags".to_string());
    }
    if flags.disable.is_some() {
        return Err("Disabling regex flags is unsupported".to_string());
    }
    Ok(())
}

fn check_node(node: &Node) -> Result<(), String> {
    match node {
        Node::Backreference => return Err("Unsupported regex node Backreference".to_string()),
        Node::Subroutine => return Err("Unsupported regex node Subroutine".to_string()),
        Node::Assertion { kind, .. } => {
            if matches!(
                kind,
                AssertionKind::SearchStart | AssertionKind::TextSegmentBoundary
            ) {
                return Err(format!("Unsupported assertion {}", kind.name()));
            }
        }
        Node::CharacterSet { kind, .. } => {
            if !matches!(
                kind,
                SetKind::Dot | SetKind::Digit | SetKind::Space | SetKind::Word
            ) {
                return Err(format!("Unsupported character set {}", kind.name()));
            }
        }
        Node::CharacterClass {
            intersection: true, ..
        } => return Err("Character-class operations are unsupported".to_string()),
        Node::Group { atomic: true, .. }
        | Node::Quantifier {
            kind: QuantifierKind::Possessive,
            ..
        } => return Err("Atomic regex matching is unsupported".to_string()),
        Node::Directive(flags) => check_flags(*flags)?,
        Node::Group { flags, .. } => check_flags(*flags)?,
        _ => {}
    }
    node.children().iter().try_for_each(check_node)
}

// ── Generator ────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Parent {
    Other,
    Class {
        negate: bool,
        first: bool,
        last: bool,
    },
    Range,
    Quantifier {
        possessive_plus: bool,
    },
}

/// Regenerates Oniguruma syntax for a sequence of nodes.
pub(super) fn generate(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|node| gen_node(node, false, Parent::Other))
        .collect()
}

fn gen_alternatives(body: &[Node], in_class: bool) -> String {
    body.iter()
        .map(|node| gen_node(node, in_class, Parent::Other))
        .collect::<Vec<_>>()
        .join("|")
}

fn gen_node(node: &Node, in_class: bool, parent: Parent) -> String {
    match node {
        Node::AbsenceFunction(body) => format!("(?~{})", gen_alternatives(body, in_class)),
        Node::Alternative(body) => body
            .iter()
            .map(|node| gen_node(node, in_class, Parent::Other))
            .collect(),
        Node::Assertion { kind, negate } => match kind {
            AssertionKind::TextSegmentBoundary => if *negate { "\\Y" } else { "\\y" }.to_string(),
            AssertionKind::WordBoundary => if *negate { "\\B" } else { "\\b" }.to_string(),
            AssertionKind::LineEnd => "$".to_string(),
            AssertionKind::LineStart => "^".to_string(),
            AssertionKind::SearchStart => "\\G".to_string(),
            AssertionKind::StringEnd => "\\z".to_string(),
            AssertionKind::StringEndNewline => "\\Z".to_string(),
            AssertionKind::StringStart => "\\A".to_string(),
        },
        Node::Backreference | Node::Subroutine => String::new(),
        Node::CapturingGroup { name, body } => {
            let enclosed = match name {
                Some(name) if name.contains('>') => format!("?'{name}'"),
                Some(name) => format!("?<{name}>"),
                None => String::new(),
            };
            format!("({enclosed}{})", gen_alternatives(body, in_class))
        }
        Node::Character(value) => gen_character(*value, in_class, parent),
        Node::CharacterClass { negate, body, .. } => {
            let len = body.len();
            let members: String = body
                .iter()
                .enumerate()
                .map(|(index, member)| {
                    gen_node(
                        member,
                        true,
                        Parent::Class {
                            negate: *negate,
                            first: index == 0,
                            last: index + 1 == len,
                        },
                    )
                })
                .collect();
            format!("[{}{members}]", if *negate { "^" } else { "" })
        }
        Node::CharacterClassRange(min, max) => format!(
            "{}-{}",
            gen_character(*min, true, Parent::Range),
            gen_character(*max, true, Parent::Range)
        ),
        Node::CharacterSet {
            kind,
            negate,
            value,
        } => {
            let value = value.as_deref().unwrap_or_default();
            match kind {
                SetKind::Any => "\\O".to_string(),
                SetKind::Digit => if *negate { "\\D" } else { "\\d" }.to_string(),
                SetKind::Dot => ".".to_string(),
                SetKind::Hex => if *negate { "\\H" } else { "\\h" }.to_string(),
                SetKind::Newline => if *negate { "\\N" } else { "\\R" }.to_string(),
                SetKind::Posix if in_class => {
                    format!("[:{}{value}:]", if *negate { "^" } else { "" })
                }
                SetKind::Posix | SetKind::Property => {
                    format!("{}{{{value}}}", if *negate { "\\P" } else { "\\p" })
                }
                SetKind::Space => if *negate { "\\S" } else { "\\s" }.to_string(),
                SetKind::TextSegment => "\\X".to_string(),
                SetKind::Word => if *negate { "\\W" } else { "\\w" }.to_string(),
            }
        }
        Node::Directive(None) => "\\K".to_string(),
        Node::Directive(Some(flags)) => {
            let rendered = flags.render();
            if rendered.is_empty() {
                String::new()
            } else {
                format!("(?{rendered})")
            }
        }
        Node::Group {
            atomic,
            flags,
            body,
        } => {
            let prefix = if *atomic {
                ">".to_string()
            } else {
                format!("{}:", flags.map(FlagModifiers::render).unwrap_or_default())
            };
            format!("(?{prefix}{})", gen_alternatives(body, in_class))
        }
        Node::Lookaround {
            behind,
            negate,
            body,
        } => format!(
            "(?{}{}{})",
            if *behind { "<" } else { "" },
            if *negate { "!" } else { "=" },
            gen_alternatives(body, in_class)
        ),
        Node::NamedCallout { kind, tag, args } => {
            let tag = tag
                .as_deref()
                .filter(|tag| !tag.is_empty())
                .map(|tag| format!("[{tag}]"))
                .unwrap_or_default();
            let args = args
                .as_ref()
                .map(|args| format!("{{{}}}", args.join(",")))
                .unwrap_or_default();
            format!("(*{}{tag}{args})", kind.to_uppercase())
        }
        Node::Quantifier {
            kind,
            min,
            max,
            body,
        } => gen_quantifier(*kind, *min, *max, body, in_class, parent),
    }
}

fn is_symbol_candidate(min: u32, max: Option<u32>) -> bool {
    (min == 0 && max == Some(1)) || (min == 0 && max.is_none()) || (min == 1 && max.is_none())
}

fn gen_quantifier(
    kind: QuantifierKind,
    min: u32,
    max: Option<u32>,
    body: &Node,
    in_class: bool,
    parent: Parent,
) -> String {
    let kid_greedy = matches!(
        body,
        Node::Quantifier {
            kind: QuantifierKind::Greedy,
            ..
        }
    );
    let kid_symbol = match body {
        Node::Quantifier { min, max, .. } => is_symbol_candidate(*min, *max),
        _ => false,
    };
    let parent_possessive_plus = matches!(
        parent,
        Parent::Quantifier {
            possessive_plus: true
        }
    );
    let forced_interval = kind == QuantifierKind::Greedy && parent_possessive_plus;
    let mut base = None;
    if is_symbol_candidate(min, max) && !forced_interval {
        if min == 0 && max == Some(1) && !kid_greedy {
            base = Some("?".to_string());
        } else if min == 0 && max.is_none() {
            base = Some("*".to_string());
        } else if min == 1
            && max.is_none()
            && (!(kid_greedy && kid_symbol) || kind == QuantifierKind::Possessive)
        {
            base = Some("+".to_string());
        }
    }
    let interval = base.is_none();
    let base = base.unwrap_or_else(|| match (kind, max) {
        (QuantifierKind::Possessive, Some(max)) => format!("{{{max},{min}}}"),
        (_, Some(max)) if max == min => format!("{{{min}}}"),
        (_, Some(max)) => format!("{{{min},{max}}}"),
        (_, None) => format!("{{{min},}}"),
    });
    let suffix = match kind {
        QuantifierKind::Greedy => "",
        QuantifierKind::Lazy => "?",
        QuantifierKind::Possessive if interval => "",
        QuantifierKind::Possessive => "+",
    };
    let body = gen_node(
        body,
        in_class,
        Parent::Quantifier {
            possessive_plus: kind == QuantifierKind::Possessive && min == 1 && max.is_none(),
        },
    );
    format!("{body}{base}{suffix}")
}

fn gen_character(value: u32, in_class: bool, parent: Parent) -> String {
    match value {
        7 => return "\\a".to_string(),
        9 => return "\\t".to_string(),
        10 => return "\\n".to_string(),
        11 => return "\\v".to_string(),
        12 => return "\\f".to_string(),
        13 => return "\\r".to_string(),
        27 => return "\\e".to_string(),
        0x2028 => return "\\u2028".to_string(),
        0x2029 => return "\\u2029".to_string(),
        0xFEFF => return "\\uFEFF".to_string(),
        _ => {}
    }
    if value < 32 || (value > 126 && value < 160) || value > 0x3FFFF {
        return if value > 0x7F {
            format!("\\x{{{value:X}}}")
        } else {
            format!("\\x{value:02X}")
        };
    }
    let Some(c) = char::from_u32(value) else {
        return format!("\\x{{{value:X}}}");
    };
    let escape = if in_class {
        let (first, last, negate) = match parent {
            Parent::Class {
                negate,
                first,
                last,
            } => (first, last, negate),
            _ => (false, false, false),
        };
        match c {
            '^' => first && !negate,
            ']' => !first,
            '-' => !first && !last,
            _ => matches!(c, '&' | '[' | '\\'),
        }
    } else {
        matches!(
            c,
            '$' | '(' | ')' | '*' | '+' | '.' | '?' | '[' | '\\' | '^' | '{' | '|'
        )
    };
    if escape {
        format!("\\{c}")
    } else {
        c.to_string()
    }
}
