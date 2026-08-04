// src/chroma/parser.rs
use crate::{Result, RuChatError};
use chroma::types::{
    BooleanOperator, CompositeExpression, ContainsOperator, DocumentExpression, DocumentOperator,
    GetResponse, Include, IncludeList, Metadata, MetadataComparison, MetadataExpression,
    MetadataSetValue, MetadataValue, PrimitiveOperator, SetOperator, SparseVector, Where,
};
use clap::Parser;
use serde::Deserialize;
use std::fmt::Display;
use std::result::Result as StdResult;

#[derive(Debug, PartialEq, Clone, Deserialize)]
enum Token {
    Identifier(String),
    Operator(String),
    Literal(String),
    And,
    Or,
    Not,
    LParen,
    RParen,
    LBracket,
    RBracket,
}

impl Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Identifier(s) | Token::Operator(s) => write!(f, "{}", s),
            Token::Literal(s) => write!(f, "'{}'", s),
            Token::And => write!(f, "AND"),
            Token::Or => write!(f, "OR"),
            Token::Not => write!(f, "NOT"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::LBracket => write!(f, "["),
            Token::RBracket => write!(f, "]"),
        }
    }
}

#[derive(Parser, Debug, Clone, PartialEq, Deserialize, Default)]
pub(crate) struct WhereArgs {
    /// The metadata query string, e.g. "key1 = 'value' AND key2 > 5",
    /// or "kind IN ['function', 'method']", or "tags NOT CONTAINS 'draft'".
    /// Supported: = != > >= < <=, IN [...], NOT IN [...], CONTAINS, NOT CONTAINS,
    /// AND, OR, parentheses. REGEX / NOT REGEX are 'document' field only.
    #[arg(short, long, help_heading = "Filtering")]
    r#where: Option<String>,
}

impl WhereArgs {
    pub(crate) fn parse(&self) -> Result<Option<Where>> {
        if let Some(ref w) = self.r#where {
            Ok(Some(parse_where(w)?))
        } else {
            Ok(None)
        }
    }
    pub(crate) fn update_from_json(&mut self, json: &serde_json::Value) -> Result<()> {
        if let Some(where_val) = json.get("where") {
            if let Some(s) = where_val.as_str() {
                self.r#where = Some(s.to_string());
                Ok(())
            } else {
                Err(RuChatError::Is(format!(
                    "Expected 'where' to be a string in JSON, got {:?}",
                    where_val
                )))
            }
        } else {
            Ok(())
        }
    }
}

fn parse_where(input: &str) -> Result<Where> {
    let tokens = tokenize(input)?;
    let mut pos = 0;
    if tokens.is_empty() {
        return Err(RuChatError::InternalError("Empty metadata query".into()));
    }
    let result = parse_expression(&tokens, &mut pos)?;
    if pos < tokens.len() {
        return Err(RuChatError::InternalError(format!(
            "Trailing tokens after: {}",
            tokens[pos]
        )));
    }
    Ok(result)
}

fn tokenize(input: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\r' | '\n' => {
                chars.next();
            }
            '(' => {
                tokens.push(Token::LParen);
                chars.next();
            }
            ')' => {
                tokens.push(Token::RParen);
                chars.next();
            }
            '[' => {
                tokens.push(Token::LBracket);
                chars.next();
            }
            ']' => {
                tokens.push(Token::RBracket);
                chars.next();
            }
            '\'' | '"' => {
                let quote = chars.next().unwrap();
                let mut s = String::new();
                while let Some(&nc) = chars.peek() {
                    if nc == quote {
                        chars.next();
                        break;
                    }
                    s.push(chars.next().unwrap());
                }
                tokens.push(Token::Literal(s));
            }
            '=' | '!' | '>' | '<' => {
                let mut op = String::new();
                op.push(chars.next().unwrap());
                if c != '='
                    && let Some(&nc) = chars.peek()
                {
                    if c == '<' && (nc == '=' || nc == '>') {
                        op.push(chars.next().unwrap());
                    }
                    if (c == '!' || c == '>') && nc == '=' {
                        op.push(chars.next().unwrap());
                    }
                }
                tokens.push(Token::Operator(op));
            }
            '-' => {
                chars.next();
                let mut s = String::from("-");
                while let Some(&nc) = chars.peek() {
                    if nc.is_ascii_digit() || nc == '.' {
                        s.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                if s == "-" {
                    return Err(RuChatError::InternalError(
                        "Unexpected '-' with no following numeric literal in query".to_string(),
                    ));
                }
                tokens.push(Token::Literal(s));
            }
            _ => {
                let mut s = String::new();
                while let Some(&nc) = chars.peek() {
                    // Allow alphanumeric, underscores, dots (for floats), and commas (for arrays)
                    if nc.is_alphanumeric() || nc == '_' || nc == '.' || nc == ',' {
                        s.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }

                if s.is_empty() {
                    return Err(RuChatError::InternalError(format!(
                        "Unexpected character '{c}' in where clause"
                    )));
                }

                match s.to_uppercase().as_str() {
                    "AND" => tokens.push(Token::And),
                    "OR" => tokens.push(Token::Or),
                    "NOT" => tokens.push(Token::Not),
                    "IN" | "CONTAINS" | "LIKE" | "REGEX" => {
                        tokens.push(Token::Operator(s.to_uppercase()))
                    }
                    _ => {
                        // Check if it's a numeric literal (starts with a digit)
                        if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                            tokens.push(Token::Literal(s));
                        } else {
                            tokens.push(Token::Identifier(s));
                        }
                    }
                }
            }
        }
    }
    Ok(tokens)
}
// OR Logic (Lowest Precedence)
fn parse_expression(tokens: &[Token], pos: &mut usize) -> Result<Where> {
    let mut left = parse_term(tokens, pos)?;
    while *pos < tokens.len() && tokens[*pos] == Token::Or {
        *pos += 1;
        let right = parse_term(tokens, pos)?;
        left = Where::Composite(CompositeExpression {
            operator: BooleanOperator::Or,
            children: vec![left, right],
        });
    }
    Ok(left)
}

// AND Logic
fn parse_term(tokens: &[Token], pos: &mut usize) -> Result<Where> {
    let mut left = parse_factor(tokens, pos)?;
    while *pos < tokens.len() && tokens[*pos] == Token::And {
        *pos += 1;
        let right = parse_factor(tokens, pos)?;
        left = Where::Composite(CompositeExpression {
            operator: BooleanOperator::And,
            children: vec![left, right],
        });
    }
    Ok(left)
}

fn parse_factor(tokens: &[Token], pos: &mut usize) -> Result<Where> {
    let current = tokens
        .get(*pos)
        .ok_or_else(|| RuChatError::InternalError("Unexpected end of input".to_string()))?;

    match current {
        Token::LParen => {
            *pos += 1;
            let expr = parse_expression(tokens, pos)?;
            if tokens.get(*pos) != Some(&Token::RParen) {
                return Err(RuChatError::InternalError(
                    "Missing closing parenthesis".to_string(),
                ));
            }
            *pos += 1;
            Ok(expr)
        }
        Token::Identifier(key) => {
            let key_name = key.clone();
            *pos += 1;

            let op = extract_operator(tokens, pos, &key_name)?;
            let val_str = extract_value(tokens, pos)?;

            if key_name.to_lowercase() == "document" {
                return Ok(Where::Document(DocumentExpression {
                    operator: map_sql_to_document_op(&op)?,
                    pattern: val_str,
                }));
            }

            Ok(Where::Metadata(MetadataExpression {
                key: key_name,
                comparison: map_sql_comparison(&op, &val_str)?,
            }))
        }
        _ => Err(RuChatError::InternalError(format!(
            "Unexpected token: {:?}",
            current
        ))),
    }
}

fn map_sql_to_document_op(op: &str) -> Result<DocumentOperator> {
    Ok(match op.to_uppercase().as_str() {
        "CONTAINS" | "LIKE" | "=" => DocumentOperator::Contains,
        "NOTCONTAINS" | "NOTLIKE" | "!=" => DocumentOperator::NotContains,
        "REGEX" => DocumentOperator::Regex,
        "NOTREGEX" => DocumentOperator::NotRegex,
        other => {
            return Err(RuChatError::InternalError(format!(
                "Unsupported operator '{other}' on 'document' field — use =, !=, CONTAINS, NOT CONTAINS, REGEX, or NOT REGEX"
            )));
        }
    })
}

// Extracted helpers to keep the parser logic clean
fn extract_operator(tokens: &[Token], pos: &mut usize, key: &str) -> Result<String> {
    let first = tokens
        .get(*pos)
        .ok_or_else(|| RuChatError::InternalError(format!("Expected operator after '{}'", key)))?;

    if *first == Token::Not {
        *pos += 1;
        let next = tokens.get(*pos).ok_or_else(|| {
            RuChatError::InternalError(format!(
                "Expected IN, CONTAINS, or REGEX after NOT (following '{key}')"
            ))
        })?;
        return match next {
            Token::Operator(o) if matches!(o.as_str(), "IN" | "CONTAINS" | "LIKE" | "REGEX") => {
                let op = format!("NOT{o}");
                *pos += 1;
                Ok(op)
            }
            _ => Err(RuChatError::InternalError(format!(
                "NOT must be followed by IN, CONTAINS, or REGEX — found {next}"
            ))),
        };
    }

    match first {
        Token::Operator(o) => {
            let op = o.clone();
            *pos += 1;
            Ok(op)
        }
        _ => Err(RuChatError::InternalError(format!(
            "Invalid operator '{}'",
            first
        ))),
    }
}

fn extract_value(tokens: &[Token], pos: &mut usize) -> Result<String> {
    let val_token = tokens
        .get(*pos)
        .ok_or_else(|| RuChatError::InternalError("Expected value after operator".to_string()))?;

    match val_token {
        Token::Literal(v) | Token::Identifier(v) => {
            *pos += 1;
            Ok(v.clone())
        }
        Token::LParen => {
            *pos += 1;
            let v = extract_value(tokens, pos)?;
            if tokens.get(*pos) == Some(&Token::RParen) {
                *pos += 1;
            }
            Ok(v)
        }
        Token::LBracket => {
            *pos += 1;
            let mut items = Vec::new();
            loop {
                match tokens.get(*pos) {
                    Some(Token::RBracket) => {
                        *pos += 1;
                        break;
                    }
                    Some(Token::Literal(v)) | Some(Token::Identifier(v)) => {
                        let trimmed = v.trim_end_matches(',');
                        if !trimmed.is_empty() {
                            items.push(trimmed.to_string());
                        }
                        *pos += 1;
                    }
                    Some(other) => {
                        return Err(RuChatError::InternalError(format!(
                            "Unexpected token '{other}' inside array literal"
                        )));
                    }
                    None => {
                        return Err(RuChatError::InternalError(
                            "Unterminated array literal — missing ']'".to_string(),
                        ));
                    }
                }
            }
            Ok(items.join(","))
        }
        _ => Err(RuChatError::InternalError(format!(
            "Expected value, found {}",
            val_token
        ))),
    }
}

fn map_sql_comparison(op: &str, val: &str) -> Result<MetadataComparison> {
    Ok(match op.to_uppercase().as_str() {
        "IN" => MetadataComparison::Set(SetOperator::In, parse_metadata_set_value(val)),
        "NOTIN" => MetadataComparison::Set(SetOperator::NotIn, parse_metadata_set_value(val)),
        "CONTAINS" | "LIKE" => {
            MetadataComparison::ArrayContains(ContainsOperator::Contains, parse_metadata_value(val))
        }
        "NOTCONTAINS" | "NOTLIKE" => MetadataComparison::ArrayContains(
            ContainsOperator::NotContains,
            parse_metadata_value(val),
        ),
        ">" => {
            MetadataComparison::Primitive(PrimitiveOperator::GreaterThan, parse_metadata_value(val))
        }
        "<" => {
            MetadataComparison::Primitive(PrimitiveOperator::LessThan, parse_metadata_value(val))
        }
        ">=" => MetadataComparison::Primitive(
            PrimitiveOperator::GreaterThanOrEqual,
            parse_metadata_value(val),
        ),
        "<=" => MetadataComparison::Primitive(
            PrimitiveOperator::LessThanOrEqual,
            parse_metadata_value(val),
        ),
        "!=" | "<>" => {
            MetadataComparison::Primitive(PrimitiveOperator::NotEqual, parse_metadata_value(val))
        }
        "=" => MetadataComparison::Primitive(PrimitiveOperator::Equal, parse_metadata_value(val)),
        "REGEX" | "NOTREGEX" => {
            return Err(RuChatError::InternalError(
                "REGEX is only supported on the 'document' field, not metadata fields".to_string(),
            ));
        }
        other => {
            return Err(RuChatError::InternalError(format!(
                "Unsupported operator '{other}' for metadata field"
            )));
        }
    })
}

/// True if `w` contains a metadata `CONTAINS`/`NOT CONTAINS` anywhere in its expression tree.
///
/// Chroma's metadata filter language has no scalar-substring operator — `MetadataComparison::
/// ArrayContains` (what `CONTAINS`/`NOT CONTAINS` always compile to, see `map_sql_comparison`
/// above) is real array-membership only. Sent to Chroma against a field whose actual stored
/// value is a plain string (`file`, `name`, ...), it silently matches nothing — a real bug
/// found from a live run using `file CONTAINS 'cli'` (one of `db_config.json`'s own documented
/// example queries) against the `repo_src` collection's scalar `file` field.
///
/// The fix is client-side: when this returns true, the caller must not send `w` to Chroma at
/// all (it would just filter everything out) and instead fetch an unfiltered/over-fetched
/// candidate set and evaluate the *whole* expression itself via `metadata_matches`, which
/// interprets `CONTAINS` correctly for both cases — a real substring check against a scalar
/// string, or a real membership check against an actual array field (so genuinely array-typed
/// fields like `references` keep working exactly as before, just evaluated client-side now
/// instead of relying on Chroma to get it right).
pub(crate) fn where_needs_client_side_eval(w: &Where) -> bool {
    match w {
        Where::Composite(c) => c.children.iter().any(where_needs_client_side_eval),
        Where::Document(_) => false,
        Where::Metadata(expr) => matches!(expr.comparison, MetadataComparison::ArrayContains(..)),
    }
}

/// Evaluates `w` against one result row's metadata. Only called when
/// `where_needs_client_side_eval` returned true for the whole expression — at that point the
/// *entire* expression is evaluated here rather than splitting it into "what Chroma already
/// filtered" plus "what we still need to check", to avoid double-counting or mis-combining
/// AND/OR semantics across two different evaluators.
///
/// `Where::Document` leaves (full-text search over document *content*, not metadata — already
/// works correctly via Chroma's native `DocumentOperator`) always evaluate to `true` here: this
/// function only exists to patch a metadata-filter gap, not to reimplement document search.
pub(crate) fn metadata_matches(w: &Where, metadata: &Metadata) -> bool {
    match w {
        Where::Composite(c) => match c.operator {
            BooleanOperator::And => c
                .children
                .iter()
                .all(|child| metadata_matches(child, metadata)),
            BooleanOperator::Or => c
                .children
                .iter()
                .any(|child| metadata_matches(child, metadata)),
        },
        Where::Document(_) => true,
        Where::Metadata(expr) => metadata_expr_matches(expr, metadata),
    }
}

fn metadata_expr_matches(expr: &MetadataExpression, metadata: &Metadata) -> bool {
    let Some(actual) = metadata.get(&expr.key) else {
        return false;
    };
    match &expr.comparison {
        MetadataComparison::Primitive(op, expected) => primitive_matches(op, actual, expected),
        MetadataComparison::Set(op, set) => set_matches(op, actual, set),
        MetadataComparison::ArrayContains(op, needle) => {
            let is_member = value_contains(actual, needle);
            match op {
                ContainsOperator::Contains => is_member,
                ContainsOperator::NotContains => !is_member,
            }
        }
    }
}

/// The actual fix: unlike Chroma's server-side `ArrayContains` (real array-membership only,
/// always false against a scalar value), this checks substring containment when `actual` is a
/// plain string, and falls back to real membership when `actual` is genuinely one of the array
/// variants — so both the originally-broken case (`file CONTAINS 'cli'`) and the
/// already-working case (`references CONTAINS 'x'`) evaluate correctly through the same path.
fn value_contains(actual: &MetadataValue, needle: &MetadataValue) -> bool {
    match (actual, needle) {
        (MetadataValue::Str(s), MetadataValue::Str(sub)) => s.contains(sub.as_str()),
        (MetadataValue::StringArray(arr), MetadataValue::Str(v)) => arr.contains(v),
        (MetadataValue::IntArray(arr), MetadataValue::Int(v)) => arr.contains(v),
        (MetadataValue::FloatArray(arr), MetadataValue::Float(v)) => arr.contains(v),
        (MetadataValue::FloatArray(arr), MetadataValue::Int(v)) => arr.contains(&(*v as f64)),
        (MetadataValue::BoolArray(arr), MetadataValue::Bool(v)) => arr.contains(v),
        _ => false,
    }
}

fn primitive_matches(
    op: &PrimitiveOperator,
    actual: &MetadataValue,
    expected: &MetadataValue,
) -> bool {
    use std::cmp::Ordering;
    let ord = compare_metadata_values(actual, expected);
    match op {
        PrimitiveOperator::Equal => ord == Some(Ordering::Equal),
        PrimitiveOperator::NotEqual => ord != Some(Ordering::Equal),
        PrimitiveOperator::GreaterThan => ord == Some(Ordering::Greater),
        PrimitiveOperator::GreaterThanOrEqual => {
            matches!(ord, Some(Ordering::Greater | Ordering::Equal))
        }
        PrimitiveOperator::LessThan => ord == Some(Ordering::Less),
        PrimitiveOperator::LessThanOrEqual => matches!(ord, Some(Ordering::Less | Ordering::Equal)),
    }
}

fn compare_metadata_values(a: &MetadataValue, b: &MetadataValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (MetadataValue::Str(x), MetadataValue::Str(y)) => Some(x.cmp(y)),
        (MetadataValue::Bool(x), MetadataValue::Bool(y)) => Some(x.cmp(y)),
        (MetadataValue::Int(x), MetadataValue::Int(y)) => Some(x.cmp(y)),
        (MetadataValue::Float(x), MetadataValue::Float(y)) => x.partial_cmp(y),
        (MetadataValue::Int(x), MetadataValue::Float(y)) => (*x as f64).partial_cmp(y),
        (MetadataValue::Float(x), MetadataValue::Int(y)) => x.partial_cmp(&(*y as f64)),
        _ => None,
    }
}

fn set_matches(op: &SetOperator, actual: &MetadataValue, set: &MetadataSetValue) -> bool {
    let is_member = match (actual, set) {
        (MetadataValue::Str(s), MetadataSetValue::Str(vs)) => vs.contains(s),
        (MetadataValue::Int(v), MetadataSetValue::Int(vs)) => vs.contains(v),
        (MetadataValue::Float(v), MetadataSetValue::Float(vs)) => {
            vs.iter().any(|x| (x - v).abs() < f64::EPSILON)
        }
        (MetadataValue::Bool(v), MetadataSetValue::Bool(vs)) => vs.contains(v),
        _ => false,
    };
    match op {
        SetOperator::In => is_member,
        SetOperator::NotIn => !is_member,
    }
}

/// Ensures `Include::Metadata` is part of the include list before a client-side-evaluated
/// query/get — `metadata_matches` has nothing to evaluate against otherwise. `None` (no
/// `--include` given) becomes `default_query` plus metadata (already part of it, but explicit
/// here rather than relying on that not changing); an explicit list keeps whatever the caller
/// asked for, just with metadata added if missing. Shared by every Chroma read path that can
/// hit `where_needs_client_side_eval` (`query.rs`, `get.rs`, `retrieve.rs`).
pub(crate) fn with_metadata_included(include: Option<IncludeList>) -> IncludeList {
    let mut list = include
        .map(|l| l.0)
        .unwrap_or_else(|| IncludeList::default_query().0);
    if !list.contains(&Include::Metadata) {
        list.push(Include::Metadata);
    }
    IncludeList(list)
}

/// Rebuilds `v` keeping only the elements at `indices`, in order — the shared primitive behind
/// every client-side `QueryResponse`/`GetResponse` filter below, since Chroma's response shape
/// is several parallel `Vec`s (ids/documents/metadatas/...) that must all be filtered by the
/// same kept-index set to stay aligned with each other.
pub(crate) fn select_indices<T: Clone>(v: &[T], indices: &[usize]) -> Vec<T> {
    indices.iter().map(|&i| v[i].clone()).collect()
}

/// Filters a `GetResponse` down to only the rows whose metadata satisfies `w` (evaluated via
/// `metadata_matches`, since Chroma couldn't apply this filter itself — see
/// `where_needs_client_side_eval`), then applies `offset`/`limit` *after* filtering rather than
/// before — a `GetResponse` isn't similarity-ranked like a query result, so unlike
/// `query.rs`'s equivalent there's no over-fetch heuristic needed, but the caller must still
/// fetch with no server-side offset/limit of its own when this path is taken (see `get.rs`/
/// `retrieve.rs`'s `execute_get`), or rows that would've matched after filtering could already
/// be cut before this ever sees them.
pub(crate) fn filter_get_response(
    r: &mut GetResponse,
    w: &Where,
    offset: usize,
    limit: Option<usize>,
) {
    let keep_indices: Vec<usize> = (0..r.ids.len())
        .filter(|&j| {
            r.metadatas
                .as_ref()
                .and_then(|m| m.get(j))
                .and_then(|opt| opt.as_ref())
                .is_some_and(|meta| metadata_matches(w, meta))
        })
        .skip(offset)
        .take(limit.unwrap_or(usize::MAX))
        .collect();

    r.ids = select_indices(&r.ids, &keep_indices);
    if let Some(v) = r.embeddings.as_mut() {
        *v = select_indices(v, &keep_indices);
    }
    if let Some(v) = r.documents.as_mut() {
        *v = select_indices(v, &keep_indices);
    }
    if let Some(v) = r.uris.as_mut() {
        *v = select_indices(v, &keep_indices);
    }
    if let Some(v) = r.metadatas.as_mut() {
        *v = select_indices(v, &keep_indices);
    }
}

fn parse_metadata_value(value_str: &str) -> MetadataValue {
    // 1. Try JSON for SparseVector support
    if value_str.starts_with('{')
        && value_str.ends_with('}')
        && let Ok(sv) = serde_json::from_str::<SparseVector>(value_str)
    {
        return MetadataValue::SparseVector(sv);
    }

    // 2. Try Primitives
    if let Ok(b) = value_str.parse::<bool>() {
        return MetadataValue::Bool(b);
    }
    if let Ok(i) = value_str.parse::<i64>() {
        return MetadataValue::Int(i);
    }
    if let Ok(f) = value_str.parse::<f64>() {
        return MetadataValue::Float(f);
    }

    // Clean brackets for array inference
    let cleaned = value_str.trim_matches(|c| c == '[' || c == ']');

    // 3. Try Arrays (Inference)
    if cleaned.contains(',') {
        let split: Vec<&str> = cleaned.split(',').map(|s| s.trim()).collect();
        if let Ok(v) = split
            .iter()
            .map(|s| s.parse::<bool>())
            .collect::<StdResult<Vec<_>, _>>()
        {
            return MetadataValue::BoolArray(v);
        }
        if let Ok(v) = split
            .iter()
            .map(|s| s.parse::<i64>())
            .collect::<StdResult<Vec<_>, _>>()
        {
            return MetadataValue::IntArray(v);
        }
        if let Ok(v) = split
            .iter()
            .map(|s| s.parse::<f64>())
            .collect::<StdResult<Vec<_>, _>>()
        {
            return MetadataValue::FloatArray(v);
        }
        return MetadataValue::StringArray(split.into_iter().map(|s| s.to_string()).collect());
    }

    // 4. Default to String
    MetadataValue::Str(value_str.to_string())
}

fn parse_metadata_set_value(value_str: &str) -> MetadataSetValue {
    let cleaned = value_str.trim_matches(|c| c == '[' || c == ']' || c == '(' || c == ')');
    let split: Vec<&str> = cleaned.split(',').map(|s| s.trim()).collect();

    if let Ok(v) = split
        .iter()
        .map(|s| s.parse::<bool>())
        .collect::<StdResult<Vec<_>, _>>()
    {
        return MetadataSetValue::Bool(v);
    }
    if let Ok(v) = split
        .iter()
        .map(|s| s.parse::<i64>())
        .collect::<StdResult<Vec<_>, _>>()
    {
        return MetadataSetValue::Int(v);
    }
    if let Ok(v) = split
        .iter()
        .map(|s| s.parse::<f64>())
        .collect::<StdResult<Vec<_>, _>>()
    {
        return MetadataSetValue::Float(v);
    }
    MetadataSetValue::Str(split.into_iter().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_tokenizer() {
        let input = "key1 = 'value' AND key2 > 5 OR document CONTAINS 'pattern'";
        let tokens = tokenize(input).unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Identifier("key1".to_string()),
                Token::Operator("=".to_string()),
                Token::Literal("value".to_string()),
                Token::And,
                Token::Identifier("key2".to_string()),
                Token::Operator(">".to_string()),
                Token::Literal("5".to_string()),
                Token::Or,
                Token::Identifier("document".to_string()),
                Token::Operator("CONTAINS".to_string()),
                Token::Literal("pattern".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_where() {
        let input = "key1 = 'value' AND key2 > 5 OR document CONTAINS 'pattern'";
        let where_clause = parse_where(input).unwrap();
        assert_eq!(
            where_clause,
            Where::Composite(CompositeExpression {
                operator: BooleanOperator::Or,
                children: vec![
                    Where::Composite(CompositeExpression {
                        operator: BooleanOperator::And,
                        children: vec![
                            Where::Metadata(MetadataExpression {
                                key: "key1".to_string(),
                                comparison: MetadataComparison::Primitive(
                                    PrimitiveOperator::Equal,
                                    MetadataValue::Str("value".to_string())
                                ),
                            }),
                            Where::Metadata(MetadataExpression {
                                key: "key2".to_string(),
                                comparison: MetadataComparison::Primitive(
                                    PrimitiveOperator::GreaterThan,
                                    MetadataValue::Int(5)
                                ),
                            }),
                        ],
                    }),
                    Where::Document(DocumentExpression {
                        operator: DocumentOperator::Contains,
                        pattern: "pattern".to_string(),
                    }),
                ],
            })
        );
    }

    #[test]
    fn test_parse_metadata_value() {
        assert_eq!(parse_metadata_value("true"), MetadataValue::Bool(true));
        assert_eq!(parse_metadata_value("123"), MetadataValue::Int(123));
        assert_eq!(parse_metadata_value("2.5"), MetadataValue::Float(2.5));
        assert_eq!(
            parse_metadata_value("a,b,c"),
            MetadataValue::StringArray(vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
        assert_eq!(
            parse_metadata_value("[1,2,3]"),
            MetadataValue::IntArray(vec![1, 2, 3])
        );
    }

    #[test]
    fn test_parse_metadata_set_value() {
        assert_eq!(
            parse_metadata_set_value("[true,false]"),
            MetadataSetValue::Bool(vec![true, false])
        );
        assert_eq!(
            parse_metadata_set_value("[1,2,3]"),
            MetadataSetValue::Int(vec![1, 2, 3])
        );
        assert_eq!(
            parse_metadata_set_value("[2.5,4.75]"),
            MetadataSetValue::Float(vec![2.5, 4.75])
        );
        assert_eq!(
            parse_metadata_set_value("[a,b,c]"),
            MetadataSetValue::Str(vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
    }

    #[test]
    fn test_map_sql_comparison() {
        assert_eq!(
            map_sql_comparison("IN", "1,2,3").unwrap(),
            MetadataComparison::Set(SetOperator::In, MetadataSetValue::Int(vec![1, 2, 3]))
        );
        assert_eq!(
            map_sql_comparison("NOTIN", "a,b,c").unwrap(),
            MetadataComparison::Set(
                SetOperator::NotIn,
                MetadataSetValue::Str(vec!["a".to_string(), "b".to_string(), "c".to_string()])
            )
        );
        assert_eq!(
            map_sql_comparison("CONTAINS", "value").unwrap(),
            MetadataComparison::ArrayContains(
                ContainsOperator::Contains,
                MetadataValue::Str("value".to_string())
            )
        );
        assert_eq!(
            map_sql_comparison("NOTCONTAINS", "value").unwrap(),
            MetadataComparison::ArrayContains(
                ContainsOperator::NotContains,
                MetadataValue::Str("value".to_string())
            )
        );
        assert_eq!(
            map_sql_comparison(">", "5").unwrap(),
            MetadataComparison::Primitive(PrimitiveOperator::GreaterThan, MetadataValue::Int(5))
        );
        assert_eq!(
            map_sql_comparison("<", "2.5").unwrap(),
            MetadataComparison::Primitive(PrimitiveOperator::LessThan, MetadataValue::Float(2.5))
        );
        assert_eq!(
            map_sql_comparison(">=", "true").unwrap(),
            MetadataComparison::Primitive(
                PrimitiveOperator::GreaterThanOrEqual,
                MetadataValue::Bool(true)
            )
        );
        assert_eq!(
            map_sql_comparison("<=", "false").unwrap(),
            MetadataComparison::Primitive(
                PrimitiveOperator::LessThanOrEqual,
                MetadataValue::Bool(false)
            )
        );
        assert_eq!(
            map_sql_comparison("!=", "value").unwrap(),
            MetadataComparison::Primitive(
                PrimitiveOperator::NotEqual,
                MetadataValue::Str("value".to_string())
            )
        );
    }

    #[test]
    fn test_map_sql_to_document_op() {
        assert_eq!(
            map_sql_to_document_op("CONTAINS").unwrap(),
            DocumentOperator::Contains
        );
        assert_eq!(
            map_sql_to_document_op("LIKE").unwrap(),
            DocumentOperator::Contains
        );
        assert_eq!(
            map_sql_to_document_op("=").unwrap(),
            DocumentOperator::Contains
        );
        assert_eq!(
            map_sql_to_document_op("NOTCONTAINS").unwrap(),
            DocumentOperator::NotContains
        );
        assert_eq!(
            map_sql_to_document_op("NOTLIKE").unwrap(),
            DocumentOperator::NotContains
        );
        assert_eq!(
            map_sql_to_document_op("!=").unwrap(),
            DocumentOperator::NotContains
        );
        assert_eq!(
            map_sql_to_document_op("REGEX").unwrap(),
            DocumentOperator::Regex
        );
        assert_eq!(
            map_sql_to_document_op("NOTREGEX").unwrap(),
            DocumentOperator::NotRegex
        );
    }

    #[test]
    fn test_extract_operator_and_value() {
        let tokens = vec![
            Token::Identifier("key".to_string()),
            Token::Operator("=".to_string()),
            Token::Literal("value".to_string()),
        ];
        let mut pos = 1; // skip "key"
        assert_eq!(extract_operator(&tokens, &mut pos, "key").unwrap(), "=");
        assert_eq!(extract_value(&tokens, &mut pos).unwrap(), "value");
    }

    #[test]
    fn test_parse_factor_document() {
        let tokens = vec![
            Token::Identifier("document".to_string()),
            Token::Operator("CONTAINS".to_string()),
            Token::Literal("pattern".to_string()),
        ];
        let mut pos = 0;
        let factor = parse_factor(&tokens, &mut pos).unwrap();
        assert_eq!(
            factor,
            Where::Document(DocumentExpression {
                operator: DocumentOperator::Contains,
                pattern: "pattern".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_factor_metadata() {
        let tokens = vec![
            Token::Identifier("key".to_string()),
            Token::Operator(">".to_string()),
            Token::Literal("5".to_string()),
        ];
        let mut pos = 0;
        let factor = parse_factor(&tokens, &mut pos).unwrap();
        assert_eq!(
            factor,
            Where::Metadata(MetadataExpression {
                key: "key".to_string(),
                comparison: MetadataComparison::Primitive(
                    PrimitiveOperator::GreaterThan,
                    MetadataValue::Int(5)
                ),
            })
        );
    }

    #[test]
    fn test_parse_factor_parentheses() {
        let tokens = vec![
            Token::LParen,
            Token::Identifier("key".to_string()),
            Token::Operator("=".to_string()),
            Token::Literal("value".to_string()),
            Token::RParen,
        ];
        let mut pos = 0;
        let factor = parse_factor(&tokens, &mut pos).unwrap();
        assert_eq!(
            factor,
            Where::Metadata(MetadataExpression {
                key: "key".to_string(),
                comparison: MetadataComparison::Primitive(
                    PrimitiveOperator::Equal,
                    MetadataValue::Str("value".to_string())
                ),
            })
        );
    }

    #[test]
    fn test_parse_term_and() {
        let tokens = vec![
            Token::Identifier("key1".to_string()),
            Token::Operator("=".to_string()),
            Token::Literal("value".to_string()),
            Token::And,
            Token::Identifier("key2".to_string()),
            Token::Operator(">".to_string()),
            Token::Literal("5".to_string()),
        ];
        let mut pos = 0;
        let term = parse_term(&tokens, &mut pos).unwrap();
        assert_eq!(
            term,
            Where::Composite(CompositeExpression {
                operator: BooleanOperator::And,
                children: vec![
                    Where::Metadata(MetadataExpression {
                        key: "key1".to_string(),
                        comparison: MetadataComparison::Primitive(
                            PrimitiveOperator::Equal,
                            MetadataValue::Str("value".to_string())
                        ),
                    }),
                    Where::Metadata(MetadataExpression {
                        key: "key2".to_string(),
                        comparison: MetadataComparison::Primitive(
                            PrimitiveOperator::GreaterThan,
                            MetadataValue::Int(5)
                        ),
                    }),
                ],
            })
        );
    }

    #[test]
    fn test_parse_expression_or() {
        let tokens = vec![
            Token::Identifier("key1".to_string()),
            Token::Operator("=".to_string()),
            Token::Literal("value".to_string()),
            Token::Or,
            Token::Identifier("key2".to_string()),
            Token::Operator(">".to_string()),
            Token::Literal("5".to_string()),
        ];
        let mut pos = 0;
        let expr = parse_expression(&tokens, &mut pos).unwrap();
        assert_eq!(
            expr,
            Where::Composite(CompositeExpression {
                operator: BooleanOperator::Or,
                children: vec![
                    Where::Metadata(MetadataExpression {
                        key: "key1".to_string(),
                        comparison: MetadataComparison::Primitive(
                            PrimitiveOperator::Equal,
                            MetadataValue::Str("value".to_string())
                        ),
                    }),
                    Where::Metadata(MetadataExpression {
                        key: "key2".to_string(),
                        comparison: MetadataComparison::Primitive(
                            PrimitiveOperator::GreaterThan,
                            MetadataValue::Int(5)
                        ),
                    }),
                ],
            })
        );
    }

    #[test]
    fn test_parse_where_complex() {
        let input = "(key1 = 'value' AND key2 > 5) OR document CONTAINS 'pattern'";
        let where_clause = parse_where(input).unwrap();
        assert_eq!(
            where_clause,
            Where::Composite(CompositeExpression {
                operator: BooleanOperator::Or,
                children: vec![
                    Where::Composite(CompositeExpression {
                        operator: BooleanOperator::And,
                        children: vec![
                            Where::Metadata(MetadataExpression {
                                key: "key1".to_string(),
                                comparison: MetadataComparison::Primitive(
                                    PrimitiveOperator::Equal,
                                    MetadataValue::Str("value".to_string())
                                ),
                            }),
                            Where::Metadata(MetadataExpression {
                                key: "key2".to_string(),
                                comparison: MetadataComparison::Primitive(
                                    PrimitiveOperator::GreaterThan,
                                    MetadataValue::Int(5)
                                ),
                            }),
                        ],
                    }),
                    Where::Document(DocumentExpression {
                        operator: DocumentOperator::Contains,
                        pattern: "pattern".to_string(),
                    }),
                ],
            })
        );
    }

    #[test]
    fn test_parse_where_empty() {
        let input = "";
        let where_clause = parse_where(input);
        assert!(where_clause.is_err());
    }

    #[test]
    fn test_parse_where_unexpected_token() {
        let input = "key1 = 'value' AND OR key2 > 5";
        let where_clause = parse_where(input);
        assert!(where_clause.is_err());
    }

    #[test]
    fn test_parse_where_missing_parenthesis() {
        let input = "(key1 = 'value' AND key2 > 5 OR document CONTAINS 'pattern'";
        let where_clause = parse_where(input);
        assert!(where_clause.is_err());
    }

    #[test]
    fn test_parse_where_trailing_tokens() {
        let input = "key1 = 'value' AND key2 > 5 extra";
        let where_clause = parse_where(input);
        assert!(where_clause.is_err());
    }

    #[test]
    fn test_parse_where_invalid_operator() {
        let input = "key1 === 'value'";
        let where_clause = parse_where(input);
        assert!(where_clause.is_err());
    }
    #[test]
    fn test_tokenize_negative_number() {
        let tokens = tokenize("count > -5").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Identifier("count".to_string()),
                Token::Operator(">".to_string()),
                Token::Literal("-5".to_string()),
            ]
        );
    }

    #[test]
    fn test_where_args_parse_none() {
        let args = WhereArgs { r#where: None };
        assert_eq!(args.parse().unwrap(), None);
    }

    #[test]
    fn test_where_args_parse_some() {
        let args = WhereArgs {
            r#where: Some("key1 = 'value'".to_string()),
        };
        assert_eq!(
            args.parse().unwrap(),
            Some(parse_where("key1 = 'value'").unwrap())
        );
    }

    #[test]
    fn test_where_args_parse_propagates_error() {
        let args = WhereArgs {
            r#where: Some("key1 === 'value'".to_string()),
        };
        assert!(args.parse().is_err());
    }

    #[test]
    fn test_where_args_update_from_json_sets_where() {
        let mut args = WhereArgs { r#where: None };
        args.update_from_json(&serde_json::json!({ "where": "key1 = 'value'" }))
            .unwrap();
        assert_eq!(args.r#where, Some("key1 = 'value'".to_string()));
    }

    #[test]
    fn test_where_args_update_from_json_missing_key_is_noop() {
        let mut args = WhereArgs { r#where: None };
        args.update_from_json(&serde_json::json!({})).unwrap();
        assert_eq!(args.r#where, None);
    }

    #[test]
    fn test_where_args_update_from_json_rejects_non_string() {
        let mut args = WhereArgs { r#where: None };
        let err = args
            .update_from_json(&serde_json::json!({ "where": 5 }))
            .unwrap_err();
        assert!(matches!(err, RuChatError::Is(_)));
    }

    fn metadata_str(pairs: &[(&str, &str)]) -> Metadata {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), MetadataValue::Str(v.to_string())))
            .collect()
    }

    // Regression: a real run found `file CONTAINS 'cli'` (one of db_config.json's own
    // documented example queries) always returned zero rows against the repo_src collection's
    // scalar `file` metadata field — because CONTAINS always compiles to Chroma's
    // ArrayContains, real array-membership only, which can never match a plain string.
    #[test]
    fn where_needs_client_side_eval_true_for_contains_on_any_field() {
        let w = parse_where("file CONTAINS 'cli'").unwrap();
        assert!(where_needs_client_side_eval(&w));
    }

    #[test]
    fn where_needs_client_side_eval_false_without_any_contains() {
        let w = parse_where("language = 'rust' AND kind IN ['function', 'method']").unwrap();
        assert!(!where_needs_client_side_eval(&w));
    }

    #[test]
    fn where_needs_client_side_eval_detects_contains_nested_inside_and_or() {
        let w = parse_where("language = 'rust' AND (kind = 'function' OR name CONTAINS 'read')")
            .unwrap();
        assert!(where_needs_client_side_eval(&w));
    }

    #[test]
    fn metadata_matches_does_real_substring_matching_on_a_scalar_string_field() {
        let w = parse_where("file CONTAINS 'cli'").unwrap();
        assert!(metadata_matches(
            &w,
            &metadata_str(&[("file", "src/cli/args.rs")])
        ));
        assert!(!metadata_matches(
            &w,
            &metadata_str(&[("file", "src/tui/io.rs")])
        ));
    }

    #[test]
    fn metadata_matches_not_contains_negates_the_substring_check() {
        let w = parse_where("file NOT CONTAINS 'cli'").unwrap();
        assert!(!metadata_matches(
            &w,
            &metadata_str(&[("file", "src/cli/args.rs")])
        ));
        assert!(metadata_matches(
            &w,
            &metadata_str(&[("file", "src/tui/io.rs")])
        ));
    }

    #[test]
    fn metadata_matches_still_does_real_membership_on_a_genuine_array_field() {
        // `references` is a StringArray in this repo's actual schema — CONTAINS against it
        // already worked correctly via Chroma before this fix, and must keep working exactly
        // the same way now that it's evaluated client-side.
        let w = parse_where("references CONTAINS 'src/foo.rs'").unwrap();
        let mut with_ref = Metadata::new();
        with_ref.insert(
            "references".to_string(),
            MetadataValue::StringArray(vec!["src/foo.rs".to_string(), "src/bar.rs".to_string()]),
        );
        assert!(metadata_matches(&w, &with_ref));

        let mut without_ref = Metadata::new();
        without_ref.insert(
            "references".to_string(),
            MetadataValue::StringArray(vec!["src/bar.rs".to_string()]),
        );
        assert!(!metadata_matches(&w, &without_ref));
    }

    #[test]
    fn metadata_matches_combines_and_correctly_with_a_client_side_contains() {
        let w = parse_where("language = 'rust' AND file CONTAINS 'cli'").unwrap();
        assert!(metadata_matches(
            &w,
            &metadata_str(&[("language", "rust"), ("file", "src/cli/args.rs")])
        ));
        // Right field name matches, but the other AND-ed condition doesn't.
        assert!(!metadata_matches(
            &w,
            &metadata_str(&[("language", "python"), ("file", "src/cli/args.rs")])
        ));
        // Language matches, but the CONTAINS substring doesn't.
        assert!(!metadata_matches(
            &w,
            &metadata_str(&[("language", "rust"), ("file", "src/tui/io.rs")])
        ));
    }

    #[test]
    fn metadata_matches_missing_key_is_false() {
        let w = parse_where("file CONTAINS 'cli'").unwrap();
        assert!(!metadata_matches(&w, &Metadata::new()));
    }

    #[test]
    fn with_metadata_included_adds_metadata_to_an_explicit_list_missing_it() {
        let list = with_metadata_included(Some(IncludeList(vec![Include::Distance])));
        assert!(list.0.contains(&Include::Metadata));
        assert!(list.0.contains(&Include::Distance));
    }

    #[test]
    fn with_metadata_included_defaults_sensibly_when_nothing_was_requested() {
        let list = with_metadata_included(None);
        assert!(list.0.contains(&Include::Metadata));
    }

    #[test]
    fn select_indices_keeps_only_the_requested_positions_in_order() {
        let v = vec!["a", "b", "c", "d"];
        assert_eq!(select_indices(&v, &[2, 0]), vec!["c", "a"]);
    }

    fn get_response_with_files(files: &[&str]) -> GetResponse {
        GetResponse {
            ids: (0..files.len()).map(|i| format!("id{i}")).collect(),
            embeddings: None,
            documents: None,
            uris: None,
            metadatas: Some(
                files
                    .iter()
                    .map(|f| Some(metadata_str(&[("file", f)])))
                    .collect(),
            ),
            include: vec![],
        }
    }

    // Regression: `get.rs`/`retrieve.rs`'s `execute_get` share the exact same CONTAINS-on-
    // scalar-field bug `query.rs` had — a `--where "file CONTAINS 'cli'"` get always returned
    // zero rows against a scalar `file` field.
    #[test]
    fn filter_get_response_keeps_only_matching_rows() {
        let w = parse_where("file CONTAINS 'cli'").unwrap();
        let mut r =
            get_response_with_files(&["src/cli/args.rs", "src/tui/io.rs", "src/cli/prompt.rs"]);
        filter_get_response(&mut r, &w, 0, None);
        assert_eq!(r.ids, vec!["id0".to_string(), "id2".to_string()]);
    }

    #[test]
    fn filter_get_response_applies_offset_and_limit_after_filtering() {
        let w = parse_where("file CONTAINS 'cli'").unwrap();
        let mut r = get_response_with_files(&[
            "src/cli/a.rs",
            "src/tui/io.rs",
            "src/cli/b.rs",
            "src/cli/c.rs",
        ]);
        filter_get_response(&mut r, &w, 1, Some(1));
        // 3 matches (id0, id2, id3) before offset/limit; skip 1, take 1 -> just id2.
        assert_eq!(r.ids, vec!["id2".to_string()]);
    }
}
