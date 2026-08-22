#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Def,
    Extern,
    If,
    Then,
    Else,
    For,
    In,
    Identifier(String),
    Number(f64),
    Op(char),
    LParen,
    RParen,
    Comma,
    Eof,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }

        if c == '#' {
            while let Some(&c) = chars.peek() {
                if c == '\n' {
                    break;
                }
                chars.next();
            }
            continue;
        }

        if c.is_ascii_digit() || c == '.' {
            let mut number = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() || c == '.' {
                    number.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            let value = number
                .parse::<f64>()
                .map_err(|_| format!("invalid number literal `{number}`"))?;
            tokens.push(Token::Number(value));
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let mut ident = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '_' {
                    ident.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            let token = match ident.as_str() {
                "def" => Token::Def,
                "extern" => Token::Extern,
                "if" => Token::If,
                "then" => Token::Then,
                "else" => Token::Else,
                "for" => Token::For,
                "in" => Token::In,
                _ => Token::Identifier(ident),
            };
            tokens.push(token);
            continue;
        }

        match c {
            '(' => {
                tokens.push(Token::LParen);
                chars.next();
            }
            ')' => {
                tokens.push(Token::RParen);
                chars.next();
            }
            ',' => {
                tokens.push(Token::Comma);
                chars.next();
            }
            '+' | '-' | '*' | '/' | '<' | '>' | '=' => {
                tokens.push(Token::Op(c));
                chars.next();
            }
            other => return Err(format!("unexpected character `{other}`")),
        }
    }

    tokens.push(Token::Eof);
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_a_function_definition() {
        let tokens = tokenize("def foo(x y) x+y").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Def,
                Token::Identifier("foo".into()),
                Token::LParen,
                Token::Identifier("x".into()),
                Token::Identifier("y".into()),
                Token::RParen,
                Token::Identifier("x".into()),
                Token::Op('+'),
                Token::Identifier("y".into()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn skips_comments() {
        let tokens = tokenize("# a comment\n42").unwrap();
        assert_eq!(tokens, vec![Token::Number(42.0), Token::Eof]);
    }

    #[test]
    fn rejects_unknown_characters() {
        assert!(tokenize("1 @ 2").is_err());
    }
}
