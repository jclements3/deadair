//! Recursive-descent parser producing the model AST.
//!
//! Syntax (Nim-flavored):
//!   let name = expr            # binding
//!   model expr                 # the solid to render
//!   expr:  a + b (union) | a - b (difference) | a * b (intersection)
//!   call:  name(a, b)  or  name a, b     # parens optional (command style)
//!   named: name(r = 8, h = 40)
//!   chain: expr.move(x, y, z).rotate(z, 90)
//!
//! Command-style call arguments are atoms (numbers/vars/parens/other calls), not
//! full operator expressions — so `cylinder 8, 40 + cube 2` reads as
//! `(cylinder 8,40) + (cube 2)`. Use parens for arithmetic inside an argument.

use super::lexer::Tok;

#[derive(Clone, Debug)]
pub enum Expr {
    Num(f64),
    Str(String),
    Var(String),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    Call(String, Vec<Arg>),
    Method(Box<Expr>, String, Vec<Arg>),
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let(String, Expr),
    Model(Expr),
}

pub struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    pub fn new(toks: Vec<Tok>) -> Self {
        Parser { toks, pos: 0 }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].clone();
        self.pos += 1;
        t
    }
    fn eat_sym(&mut self, c: char) -> bool {
        if self.peek() == &Tok::Sym(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn skip_newlines(&mut self) {
        while self.peek() == &Tok::Newline {
            self.pos += 1;
        }
    }

    pub fn parse_program(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while self.peek() != &Tok::Eof {
            stmts.push(self.parse_stmt()?);
            // Statements end at a newline or EOF.
            if self.peek() != &Tok::Eof && self.peek() != &Tok::Newline {
                return Err(format!("unexpected {:?} after statement", self.peek()));
            }
            self.skip_newlines();
        }
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        if let Tok::Ident(kw) = self.peek().clone() {
            if kw == "let" {
                self.bump();
                let name = self.expect_ident()?;
                if !self.eat_sym('=') {
                    return Err(format!("expected '=' in let binding for '{name}'"));
                }
                let e = self.parse_expr(0)?;
                return Ok(Stmt::Let(name, e));
            }
            if kw == "model" {
                self.bump();
                let e = self.parse_expr(0)?;
                return Ok(Stmt::Model(e));
            }
        }
        Err("expected a statement: `let name = ...` or `model ...`".into())
    }

    /// Parse a single standalone expression, requiring it to consume all input.
    /// Used to evaluate numeric relationship expressions outside a full program.
    pub fn parse_single(&mut self) -> Result<Expr, String> {
        self.skip_newlines();
        let e = self.parse_expr(0)?;
        self.skip_newlines();
        if self.peek() != &Tok::Eof {
            return Err(format!("unexpected {:?} in expression", self.peek()));
        }
        Ok(e)
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.bump() {
            Tok::Ident(s) => Ok(s),
            other => Err(format!("expected identifier, found {other:?}")),
        }
    }

    // Pratt-style binary expression parsing. `+ -` bind at 10, `*` at 20.
    fn parse_expr(&mut self, min_bp: u8) -> Result<Expr, String> {
        let mut left = self.parse_prefix()?;
        loop {
            let (op, bp) = match self.peek() {
                Tok::Sym('+') => ('+', 10),
                Tok::Sym('-') => ('-', 10),
                Tok::Sym('*') => ('*', 20),
                Tok::Sym('/') => ('/', 20),
                _ => break,
            };
            if bp < min_bp {
                break;
            }
            self.bump();
            let right = self.parse_expr(bp + 1)?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_prefix(&mut self) -> Result<Expr, String> {
        if self.eat_sym('-') {
            let e = self.parse_prefix()?;
            return Ok(Expr::Neg(Box::new(e)));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.parse_primary()?;
        while self.peek() == &Tok::Sym('.') {
            self.bump();
            let method = self.expect_ident()?;
            let args = self.parse_call_args()?;
            e = Expr::Method(Box::new(e), method, args);
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.bump() {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Sym('(') => {
                let e = self.parse_expr(0)?;
                if !self.eat_sym(')') {
                    return Err("expected ')'".into());
                }
                Ok(e)
            }
            Tok::Ident(name) => {
                if self.peek() == &Tok::Sym('(') || self.starts_command_arg() {
                    let args = self.parse_call_args()?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Var(name))
                }
            }
            other => Err(format!("unexpected {other:?}")),
        }
    }

    /// True if the next token could begin a bare (parenless) argument list.
    /// Deliberately excludes `-`, so `a - b` always reads as subtraction rather
    /// than `a(-b)`; use parentheses for a leading negative argument.
    fn starts_command_arg(&self) -> bool {
        matches!(self.peek(), Tok::Num(_) | Tok::Str(_) | Tok::Ident(_))
    }

    /// A command-style argument: unary minus + primary, but NOT a `.method`
    /// chain — so `cube 3 .move …` attaches `.move` to the call, not to `3`.
    fn parse_atom(&mut self) -> Result<Expr, String> {
        if self.eat_sym('-') {
            Ok(Expr::Neg(Box::new(self.parse_atom()?)))
        } else {
            self.parse_primary()
        }
    }

    /// Parse either `(a, b)` or a bare `a, b` argument list. Bare args are atoms.
    fn parse_call_args(&mut self) -> Result<Vec<Arg>, String> {
        let mut args = Vec::new();
        if self.eat_sym('(') {
            if self.eat_sym(')') {
                return Ok(args);
            }
            loop {
                args.push(self.parse_arg(true)?);
                if self.eat_sym(',') {
                    continue;
                }
                if self.eat_sym(')') {
                    break;
                }
                return Err("expected ',' or ')' in argument list".into());
            }
        } else if self.starts_command_arg() {
            loop {
                args.push(self.parse_arg(false)?);
                if self.eat_sym(',') {
                    continue;
                }
                break;
            }
        }
        Ok(args)
    }

    /// One argument, optionally `name = value`. In paren calls the value may be a
    /// full expression; in command calls it is an atom (prefix expression).
    fn parse_arg(&mut self, full_expr: bool) -> Result<Arg, String> {
        // Look for `ident =` (named argument).
        if let Tok::Ident(name) = self.peek().clone() {
            if self.toks.get(self.pos + 1) == Some(&Tok::Sym('=')) {
                self.pos += 2; // consume ident and '='
                let value = if full_expr {
                    self.parse_expr(0)?
                } else {
                    self.parse_atom()?
                };
                return Ok(Arg {
                    name: Some(name),
                    value,
                });
            }
        }
        let value = if full_expr {
            self.parse_expr(0)?
        } else {
            self.parse_atom()?
        };
        Ok(Arg { name: None, value })
    }
}
