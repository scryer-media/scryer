use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

pub(crate) const MAX_TOKENS: usize = 256;
pub(crate) const MAX_BRACKET_DEPTH: usize = 8;

/// Byte span inside the sanitized input string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
}

/// Separator kind preserved between tokens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeparatorKind {
    #[default]
    Boundary,
    Dot,
    Underscore,
    Space,
    Hyphen,
    Slash,
    OpenBracket,
    CloseBracket,
    OpenParen,
    CloseParen,
    OpenBrace,
    CloseBrace,
    Other,
}

/// Bracket type used in CST grouping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BracketKind {
    #[default]
    Square,
    Paren,
    Brace,
}

/// Lossless token produced by the lexer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub raw: String,
    pub normalized: String,
    pub span: TextSpan,
    pub separator_before: SeparatorKind,
    pub separator_after: SeparatorKind,
    pub group_id: Option<usize>,
    pub bracket_depth: u8,
}

/// Lightweight CST for grouped runs and bracket clusters.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseCst {
    pub nodes: Vec<CstNode>,
}

/// CST nodes built from the lossless token stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CstNode {
    Token {
        token_index: usize,
    },
    BracketGroup {
        group_id: usize,
        bracket_kind: BracketKind,
        token_indices: Vec<usize>,
    },
    HyphenGroup {
        token_indices: Vec<usize>,
    },
    SlashGroup {
        token_indices: Vec<usize>,
    },
    DelimitedRun {
        separator: SeparatorKind,
        token_indices: Vec<usize>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct LexedRelease {
    pub(crate) tokens: Vec<Token>,
    pub(crate) cst: ReleaseCst,
    pub(crate) hints: Vec<String>,
}

pub(crate) fn lex_lossless(input: &str) -> LexedRelease {
    let mut tokens = Vec::new();
    let mut hints = Vec::new();
    let mut token_start = None::<usize>;
    let mut separator_before = SeparatorKind::Boundary;
    let mut group_stack = Vec::<(usize, BracketKind)>::new();
    let mut next_group_id = 0usize;
    let mut chars = input.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        if is_separator(ch) {
            let flushed_token = token_start.is_some();
            if let Some(start) = token_start.take() {
                let end = index;
                if let Some(token) = build_token(
                    input,
                    start,
                    end,
                    separator_before,
                    separator_from_char(ch),
                    group_stack.last().map(|(group_id, _)| *group_id),
                    group_stack.len(),
                ) {
                    tokens.push(token);
                    if tokens.len() == MAX_TOKENS {
                        hints.push("token_limit_reached".to_string());
                        break;
                    }
                }
            }

            separator_before = if flushed_token {
                separator_from_char(ch)
            } else {
                merge_separator_before(separator_before, separator_from_char(ch))
            };
            match ch {
                '[' => push_group(&mut group_stack, &mut next_group_id, BracketKind::Square),
                '(' => push_group(&mut group_stack, &mut next_group_id, BracketKind::Paren),
                '{' => push_group(&mut group_stack, &mut next_group_id, BracketKind::Brace),
                ']' | ')' | '}' => {
                    group_stack.pop();
                }
                _ => {}
            }
            if group_stack.len() > MAX_BRACKET_DEPTH {
                group_stack.truncate(MAX_BRACKET_DEPTH);
                hints.push("bracket_depth_flattened".to_string());
            }
            continue;
        }

        if token_start.is_none() {
            token_start = Some(index);
        }

        if chars.peek().is_none()
            && let Some(start) = token_start.take()
            && let Some(token) = build_token(
                input,
                start,
                input.len(),
                separator_before,
                SeparatorKind::Boundary,
                group_stack.last().map(|(group_id, _)| *group_id),
                group_stack.len(),
            )
        {
            tokens.push(token);
        }
    }

    let cst = build_cst(&tokens);
    LexedRelease { tokens, cst, hints }
}

fn build_token(
    input: &str,
    start: usize,
    end: usize,
    separator_before: SeparatorKind,
    separator_after: SeparatorKind,
    group_id: Option<usize>,
    bracket_depth: usize,
) -> Option<Token> {
    let raw = input.get(start..end)?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(Token {
        raw: raw.to_string(),
        normalized: normalize_token(raw),
        span: TextSpan { start, end },
        separator_before,
        separator_after,
        group_id,
        bracket_depth: bracket_depth.min(MAX_BRACKET_DEPTH) as u8,
    })
}

fn build_cst(tokens: &[Token]) -> ReleaseCst {
    let mut nodes = tokens
        .iter()
        .enumerate()
        .map(|(token_index, _)| CstNode::Token { token_index })
        .collect::<Vec<_>>();

    let mut group_map = std::collections::BTreeMap::<usize, (BracketKind, Vec<usize>)>::new();
    for (token_index, token) in tokens.iter().enumerate() {
        if let Some(group_id) = token.group_id {
            let bracket_kind = match token.separator_before {
                SeparatorKind::OpenParen | SeparatorKind::CloseParen => BracketKind::Paren,
                SeparatorKind::OpenBrace | SeparatorKind::CloseBrace => BracketKind::Brace,
                _ => BracketKind::Square,
            };
            group_map
                .entry(group_id)
                .or_insert_with(|| (bracket_kind, Vec::new()))
                .1
                .push(token_index);
        }
    }
    for (group_id, (bracket_kind, token_indices)) in group_map {
        nodes.push(CstNode::BracketGroup {
            group_id,
            bracket_kind,
            token_indices,
        });
    }

    for separator in [
        SeparatorKind::Hyphen,
        SeparatorKind::Slash,
        SeparatorKind::Dot,
        SeparatorKind::Underscore,
        SeparatorKind::Space,
    ] {
        let mut current = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            if index > 0 && token.separator_before == separator {
                current.push(index - 1);
                current.push(index);
            } else if !current.is_empty() {
                current.sort_unstable();
                current.dedup();
                nodes.push(match separator {
                    SeparatorKind::Hyphen => CstNode::HyphenGroup {
                        token_indices: current.clone(),
                    },
                    SeparatorKind::Slash => CstNode::SlashGroup {
                        token_indices: current.clone(),
                    },
                    _ => CstNode::DelimitedRun {
                        separator,
                        token_indices: current.clone(),
                    },
                });
                current.clear();
            }
        }
        if !current.is_empty() {
            current.sort_unstable();
            current.dedup();
            nodes.push(match separator {
                SeparatorKind::Hyphen => CstNode::HyphenGroup {
                    token_indices: current,
                },
                SeparatorKind::Slash => CstNode::SlashGroup {
                    token_indices: current,
                },
                _ => CstNode::DelimitedRun {
                    separator,
                    token_indices: current,
                },
            });
        }
    }

    ReleaseCst { nodes }
}

pub(crate) fn normalize_token(raw: &str) -> String {
    // Latin accent folding is useful to the technical/context token parser.
    // Decomposing every script would also erase kana voicing and Cyrillic
    // breves. Compose native text first and only decompose Latin letters.
    // Identity comparisons use raw title spans, not these lossy tokens.
    let mut normalized = String::new();
    for ch in raw.nfkc() {
        match ch {
            'æ' | 'Æ' => normalized.push_str("AE"),
            'œ' | 'Œ' => normalized.push_str("OE"),
            'ø' | 'Ø' => normalized.push('O'),
            'ł' | 'Ł' => normalized.push('L'),
            'đ' | 'Đ' => normalized.push('D'),
            '+' => normalized.push('+'),
            _ if matches!(ch as u32, 0xc0..=0x24f | 0x1e00..=0x1eff) => {
                for letter in ch
                    .to_string()
                    .nfkd()
                    .filter(|letter| letter.is_alphanumeric())
                {
                    normalized.extend(letter.to_uppercase());
                }
            }
            _ if ch.is_alphanumeric() => normalized.extend(ch.to_uppercase()),
            _ => {}
        }
    }
    normalized
}

fn push_group(
    group_stack: &mut Vec<(usize, BracketKind)>,
    next_group_id: &mut usize,
    bracket_kind: BracketKind,
) {
    let group_id = *next_group_id;
    *next_group_id += 1;
    group_stack.push((group_id, bracket_kind));
}

fn is_separator(ch: char) -> bool {
    matches!(
        ch,
        '.' | '_'
            | ' '
            | '-'
            | '–'
            | '—'
            | '−'
            | '~'
            | '～'
            | '/'
            | ':'
            | '['
            | ']'
            | '('
            | ')'
            | '{'
            | '}'
            | '\t'
    )
}

fn separator_from_char(ch: char) -> SeparatorKind {
    match ch {
        '.' => SeparatorKind::Dot,
        '_' => SeparatorKind::Underscore,
        ' ' | '\t' => SeparatorKind::Space,
        '-' | '–' | '—' | '−' | '~' | '～' => SeparatorKind::Hyphen,
        '/' => SeparatorKind::Slash,
        ':' => SeparatorKind::Other,
        '[' => SeparatorKind::OpenBracket,
        ']' => SeparatorKind::CloseBracket,
        '(' => SeparatorKind::OpenParen,
        ')' => SeparatorKind::CloseParen,
        '{' => SeparatorKind::OpenBrace,
        '}' => SeparatorKind::CloseBrace,
        _ => SeparatorKind::Other,
    }
}

fn merge_separator_before(previous: SeparatorKind, next: SeparatorKind) -> SeparatorKind {
    if next == SeparatorKind::Space
        && !matches!(previous, SeparatorKind::Boundary | SeparatorKind::Space)
    {
        previous
    } else {
        next
    }
}
