#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Variable(String),
    Binary {
        op: char,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Call {
        callee: String,
        args: Vec<Expr>,
    },
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    For {
        var_name: String,
        start: Box<Expr>,
        end: Box<Expr>,
        step: Option<Box<Expr>>,
        body: Box<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prototype {
    pub name: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub proto: Prototype,
    /// `None` for `extern` declarations (no body to codegen).
    pub body: Option<Expr>,
}

/// One REPL/top-level unit: a definition, an extern declaration, or a bare
/// expression (which the parser wraps into an anonymous function so it can
/// be codegen'd and JIT-called like any other function).
#[derive(Debug, Clone, PartialEq)]
pub enum TopLevel {
    Function(Function),
    Extern(Prototype),
    Expression(Expr),
}
