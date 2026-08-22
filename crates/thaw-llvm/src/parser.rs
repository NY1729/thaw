use crate::ast::{Expr, Function, Prototype, TopLevel};
use crate::lexer::Token;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

fn binop_precedence(op: char) -> i32 {
    match op {
        '<' | '>' => 10,
        '+' | '-' => 20,
        '*' | '/' => 40,
        _ => -1,
    }
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        if self.peek() == expected {
            self.advance();
            Ok(())
        } else {
            Err(format!("expected {expected:?}, found {:?}", self.peek()))
        }
    }

    /// Parses every top-level item until EOF.
    pub fn parse_program(&mut self) -> Result<Vec<TopLevel>, String> {
        let mut items = Vec::new();
        while *self.peek() != Token::Eof {
            items.push(self.parse_top_level()?);
        }
        Ok(items)
    }

    fn parse_top_level(&mut self) -> Result<TopLevel, String> {
        match self.peek() {
            Token::Def => Ok(TopLevel::Function(self.parse_definition()?)),
            Token::Extern => Ok(TopLevel::Extern(self.parse_extern()?)),
            _ => Ok(TopLevel::Expression(self.parse_expression()?)),
        }
    }

    fn parse_definition(&mut self) -> Result<Function, String> {
        self.expect(&Token::Def)?;
        let proto = self.parse_prototype()?;
        let body = self.parse_expression()?;
        Ok(Function {
            proto,
            body: Some(body),
        })
    }

    fn parse_extern(&mut self) -> Result<Prototype, String> {
        self.expect(&Token::Extern)?;
        self.parse_prototype()
    }

    fn parse_prototype(&mut self) -> Result<Prototype, String> {
        let name = match self.advance() {
            Token::Identifier(name) => name,
            other => return Err(format!("expected function name, found {other:?}")),
        };

        self.expect(&Token::LParen)?;
        let mut args = Vec::new();
        while let Token::Identifier(arg) = self.peek().clone() {
            args.push(arg);
            self.advance();
        }
        self.expect(&Token::RParen)?;

        Ok(Prototype { name, args })
    }

    fn parse_expression(&mut self) -> Result<Expr, String> {
        let lhs = self.parse_primary()?;
        self.parse_bin_op_rhs(0, lhs)
    }

    fn parse_bin_op_rhs(&mut self, expr_prec: i32, mut lhs: Expr) -> Result<Expr, String> {
        loop {
            let op = match self.peek() {
                Token::Op(c) => *c,
                _ => return Ok(lhs),
            };
            let tok_prec = binop_precedence(op);
            if tok_prec < expr_prec {
                return Ok(lhs);
            }

            self.advance();
            let mut rhs = self.parse_primary()?;

            let next_op = match self.peek() {
                Token::Op(c) => Some(*c),
                _ => None,
            };
            if let Some(next_op) = next_op {
                if tok_prec < binop_precedence(next_op) {
                    rhs = self.parse_bin_op_rhs(tok_prec + 1, rhs)?;
                }
            }

            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.peek().clone() {
            Token::Number(n) => {
                self.advance();
                Ok(Expr::Number(n))
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            Token::Identifier(name) => {
                self.advance();
                if *self.peek() != Token::LParen {
                    return Ok(Expr::Variable(name));
                }
                self.advance();
                let mut args = Vec::new();
                if *self.peek() != Token::RParen {
                    loop {
                        args.push(self.parse_expression()?);
                        if *self.peek() == Token::Comma {
                            self.advance();
                            continue;
                        }
                        break;
                    }
                }
                self.expect(&Token::RParen)?;
                Ok(Expr::Call { callee: name, args })
            }
            Token::If => self.parse_if(),
            Token::For => self.parse_for(),
            other => Err(format!("unexpected token {other:?}")),
        }
    }

    fn parse_if(&mut self) -> Result<Expr, String> {
        self.expect(&Token::If)?;
        let cond = self.parse_expression()?;
        self.expect(&Token::Then)?;
        let then_branch = self.parse_expression()?;
        self.expect(&Token::Else)?;
        let else_branch = self.parse_expression()?;
        Ok(Expr::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(else_branch),
        })
    }

    fn parse_for(&mut self) -> Result<Expr, String> {
        self.expect(&Token::For)?;
        let var_name = match self.advance() {
            Token::Identifier(name) => name,
            other => return Err(format!("expected loop variable name, found {other:?}")),
        };
        self.expect(&Token::Op('='))?;
        let start = self.parse_expression()?;
        self.expect(&Token::Comma)?;
        let end = self.parse_expression()?;

        let step = if *self.peek() == Token::Comma {
            self.advance();
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };

        self.expect(&Token::In)?;
        let body = self.parse_expression()?;

        Ok(Expr::For {
            var_name,
            start: Box::new(start),
            end: Box::new(end),
            step,
            body: Box::new(body),
        })
    }
}

pub fn parse(input: &str) -> Result<Vec<TopLevel>, String> {
    let tokens = crate::lexer::tokenize(input)?;
    Parser::new(tokens).parse_program()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_definition() {
        let program = parse("def foo(x y) x+y").unwrap();
        assert_eq!(
            program,
            vec![TopLevel::Function(Function {
                proto: Prototype {
                    name: "foo".into(),
                    args: vec!["x".into(), "y".into()],
                },
                body: Some(Expr::Binary {
                    op: '+',
                    lhs: Box::new(Expr::Variable("x".into())),
                    rhs: Box::new(Expr::Variable("y".into())),
                }),
            })]
        );
    }

    #[test]
    fn respects_operator_precedence() {
        let program = parse("1+2*3").unwrap();
        assert_eq!(
            program,
            vec![TopLevel::Expression(Expr::Binary {
                op: '+',
                lhs: Box::new(Expr::Number(1.0)),
                rhs: Box::new(Expr::Binary {
                    op: '*',
                    lhs: Box::new(Expr::Number(2.0)),
                    rhs: Box::new(Expr::Number(3.0)),
                }),
            })]
        );
    }

    #[test]
    fn parses_call_expression() {
        let program = parse("foo(1, bar(2))").unwrap();
        assert_eq!(
            program,
            vec![TopLevel::Expression(Expr::Call {
                callee: "foo".into(),
                args: vec![
                    Expr::Number(1.0),
                    Expr::Call {
                        callee: "bar".into(),
                        args: vec![Expr::Number(2.0)],
                    },
                ],
            })]
        );
    }

    #[test]
    fn parses_extern_declaration() {
        let program = parse("extern sin(x)").unwrap();
        assert_eq!(
            program,
            vec![TopLevel::Extern(Prototype {
                name: "sin".into(),
                args: vec!["x".into()],
            })]
        );
    }
}
