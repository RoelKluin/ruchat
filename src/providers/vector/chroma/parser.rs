// src/chroma/parser.rs
use crate::RuChatError;
use chroma::types::{
    BooleanOperator, CompositeExpression, DocumentExpression, DocumentOperator,
    MetadataComparison, MetadataExpression, MetadataValue, PrimitiveOperator, Where,
    MetadataSetValue, SetOperator, ContainsOperator, SparseVector,
};
use std::fmt::Display;

#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    Identifier(String),
    Operator(String),
    Literal(String),
    And, Or, LParen, RParen,
}

impl Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Identifier(s) | Token::Operator(s) => write!(f, "{}", s),
            Token::Literal(s) => write!(f, "'{}'", s),
            Token::And => write!(f, "AND"),
            Token::Or => write!(f, "OR"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
        }
    }
}


pub fn parse_where(input: &str) -> Result<Where, RuChatError> {
    let tokens = tokenize(input);
    let mut pos = 0;
    if tokens.is_empty() {
        return Err(RuChatError::InternalError("Empty metadata query".into()));
    }
    let result = parse_expression(&tokens, &mut pos)?;
    if pos < tokens.len() {
        return Err(RuChatError::InternalError(format!("Trailing tokens after: {}", tokens[pos])));
    }
    Ok(result)
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\r' | '\n' => { chars.next(); }
            '(' => { tokens.push(Token::LParen); chars.next(); }
            ')' => { tokens.push(Token::RParen); chars.next(); }
            '\'' | '"' => {
                let quote = chars.next().unwrap();
                let mut s = String::new();
                while let Some(&nc) = chars.peek() {
                    if nc == quote { chars.next(); break; }
                    s.push(chars.next().unwrap());
                }
                tokens.push(Token::Literal(s));
            }
            '=' | '!' | '>' | '<' => {
                let mut op = String::new();
                op.push(chars.next().unwrap());
                if let Some(&nc) = chars.peek() {
                    if (c == '!' || c == '>' || c == '<') && nc == '=' { op.push(chars.next().unwrap()); }
                    else if c == '<' && nc == '>' { op.push(chars.next().unwrap()); }
                }
                tokens.push(Token::Operator(op));
            }
            _ => {
                let mut s = String::new();
                while let Some(&nc) = chars.peek() {
                    if nc.is_alphanumeric() || nc == '_' { s.push(chars.next().unwrap()); }
                    else { break; }
                }
                match s.to_uppercase().as_str() {
                    "AND" => tokens.push(Token::And),
                    "OR" => tokens.push(Token::Or),
                    "IN" => tokens.push(Token::Operator("IN".to_string())),
                    _ => tokens.push(Token::Identifier(s)),
                }
            }
        }
    }
    tokens
}
// OR Logic (Lowest Precedence)
fn parse_expression(tokens: &[Token], pos: &mut usize) -> Result<Where, RuChatError> {
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
fn parse_term(tokens: &[Token], pos: &mut usize) -> Result<Where, RuChatError> {
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

fn parse_factor(tokens: &[Token], pos: &mut usize) -> Result<Where, RuChatError> {
    let current = tokens.get(*pos).ok_or_else(|| {
        RuChatError::InternalError("Unexpected end of input".to_string())
    })?;

    match current {
        Token::LParen => {
            *pos += 1;
            let expr = parse_expression(tokens, pos)?;
            if tokens.get(*pos) != Some(&Token::RParen) {
                return Err(RuChatError::InternalError("Missing closing parenthesis".to_string()));
            }
            *pos += 1;
            Ok(expr)
        }
        Token::Identifier(key) => {
            let key_name = key.clone();
            *pos += 1;

            let op = extract_operator(tokens, pos, &key_name)?;
            let val_str = extract_value(tokens, pos)?;

            // RESTORED: Special handling for 'document' keyword
            if key_name.to_lowercase() == "document" {
                return Ok(Where::Document(DocumentExpression {
                    operator: map_sql_to_document_op(&op),
                    pattern: val_str,
                }));
            }

            // Metadata handling
            Ok(Where::Metadata(MetadataExpression {
                key: key_name,
                comparison: map_sql_comparison(&op, &val_str),
            }))
        }
        _ => Err(RuChatError::InternalError(format!("Unexpected token: {:?}", current))),
    }
}

// New helper for Document operators
fn map_sql_to_document_op(op: &str) -> DocumentOperator {
    match op.to_uppercase().as_str() {
        "CONTAINS" | "LIKE" | "=" => DocumentOperator::Contains,
        "NOTCONTAINS" | "NOTLIKE" | "!=" => DocumentOperator::NotContains,
        "REGEX" => DocumentOperator::Regex,
        "NOTREGEX" => DocumentOperator::NotRegex,
        _ => DocumentOperator::Contains,
    }
}

// Extracted helpers to keep the parser logic clean
fn extract_operator(tokens: &[Token], pos: &mut usize, key: &str) -> Result<String, RuChatError> {
    let op_token = tokens.get(*pos).ok_or_else(|| {
        RuChatError::InternalError(format!("Expected operator after '{}'", key))
    })?;
    let op = match op_token {
        Token::Operator(o) => o.clone(),
        _ => return Err(RuChatError::InternalError(format!("Invalid operator '{}'", op_token))),
    };
    *pos += 1;
    Ok(op)
}

fn extract_value(tokens: &[Token], pos: &mut usize) -> Result<String, RuChatError> {
    let val_token = tokens.get(*pos).ok_or_else(|| {
        RuChatError::InternalError("Expected value after operator".to_string())
    })?;
    let val = match val_token {
        Token::Literal(v) | Token::Identifier(v) => v.clone(),
        _ => return Err(RuChatError::InternalError("Expected literal value".to_string())),
    };
    *pos += 1;
    Ok(val)
}

fn map_sql_comparison(op: &str, val: &str) -> MetadataComparison {
    match op.to_uppercase().as_str() {
        "IN" => MetadataComparison::Set(SetOperator::In, parse_metadata_set_value(val)),
        "NOTIN" => MetadataComparison::Set(SetOperator::NotIn, parse_metadata_set_value(val)),
        "CONTAINS" => MetadataComparison::ArrayContains(ContainsOperator::Contains, parse_metadata_value(val)),
        "NOTCONTAINS" => MetadataComparison::ArrayContains(ContainsOperator::NotContains, parse_metadata_value(val)),
        ">" => MetadataComparison::Primitive(PrimitiveOperator::GreaterThan, parse_metadata_value(val)),
        "<" => MetadataComparison::Primitive(PrimitiveOperator::LessThan, parse_metadata_value(val)),
        ">=" => MetadataComparison::Primitive(PrimitiveOperator::GreaterThanOrEqual, parse_metadata_value(val)),
        "<=" => MetadataComparison::Primitive(PrimitiveOperator::LessThanOrEqual, parse_metadata_value(val)),
        "!=" | "<>" => MetadataComparison::Primitive(PrimitiveOperator::NotEqual, parse_metadata_value(val)),
        _ => MetadataComparison::Primitive(PrimitiveOperator::Equal, parse_metadata_value(val)),
    }
}

fn parse_metadata_value(value_str: &str) -> MetadataValue {
    // 1. Try JSON for SparseVector support
    if value_str.starts_with('{') && value_str.ends_with('}') {
        if let Ok(sv) = serde_json::from_str::<SparseVector>(value_str) {
            return MetadataValue::SparseVector(sv);
        }
    }

    // 2. Try Primitives
    if let Ok(b) = value_str.parse::<bool>() { return MetadataValue::Bool(b); }
    if let Ok(i) = value_str.parse::<i64>() { return MetadataValue::Int(i); }
    if let Ok(f) = value_str.parse::<f64>() { return MetadataValue::Float(f); }

    // 3. Try Arrays (Inference)
    if value_str.contains(',') {
        let split: Vec<&str> = value_str.split(',').map(|s| s.trim()).collect();
        if let Ok(v) = split.iter().map(|s| s.parse::<bool>()).collect::<Result<Vec<_>, _>>() {
            return MetadataValue::BoolArray(v);
        }
        if let Ok(v) = split.iter().map(|s| s.parse::<i64>()).collect::<Result<Vec<_>, _>>() {
            return MetadataValue::IntArray(v);
        }
        if let Ok(v) = split.iter().map(|s| s.parse::<f64>()).collect::<Result<Vec<_>, _>>() {
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

    if let Ok(v) = split.iter().map(|s| s.parse::<bool>()).collect::<Result<Vec<_>, _>>() {
        return MetadataSetValue::Bool(v);
    }
    if let Ok(v) = split.iter().map(|s| s.parse::<i64>()).collect::<Result<Vec<_>, _>>() {
        return MetadataSetValue::Int(v);
    }
    if let Ok(v) = split.iter().map(|s| s.parse::<f64>()).collect::<Result<Vec<_>, _>>() {
        return MetadataSetValue::Float(v);
    }
    MetadataSetValue::Str(split.into_iter().map(|s| s.to_string()).collect())
}
