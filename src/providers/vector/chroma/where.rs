// src/chroma/parser.rs
use crate::{Result, RuChatError};
use chroma::types::{
    BooleanOperator, CompositeExpression, ContainsOperator, DocumentExpression, DocumentOperator,
    MetadataComparison, MetadataExpression, MetadataSetValue, MetadataValue, PrimitiveOperator,
    SetOperator, SparseVector, Where,
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
            if where_val.is_string() {
                self.r#where = Some(where_val.as_str().unwrap().to_string());
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
        _ => Err(RuChatError::InternalError(format!("Invalid operator '{}'", first))),
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
        assert_eq!(map_sql_to_document_op("LIKE").unwrap(), DocumentOperator::Contains);
        assert_eq!(map_sql_to_document_op("=").unwrap(), DocumentOperator::Contains);
        assert_eq!(
            map_sql_to_document_op("NOTCONTAINS").unwrap(),
            DocumentOperator::NotContains
        );
        assert_eq!(
            map_sql_to_document_op("NOTLIKE").unwrap(),
            DocumentOperator::NotContains
        );
        assert_eq!(map_sql_to_document_op("!=").unwrap(), DocumentOperator::NotContains);
        assert_eq!(map_sql_to_document_op("REGEX").unwrap(), DocumentOperator::Regex);
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
        assert_eq!(args.parse().unwrap(), Some(parse_where("key1 = 'value'").unwrap()));
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
}
