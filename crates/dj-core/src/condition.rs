//! Conditions for run-until calls and search jobs (docs/mcp.md#conditions).
//!
//! A small expression language over memory and coverage:
//!
//! ```text
//! RAM:0x44 >= 2                 a byte of a memory region
//! u16(RAM:0x10) > 1000          two bytes, little-endian
//! ran(PRG:0x1F3A)               that ROM byte ran as an opcode since the call started
//! frame >= 300                  frames since the call started
//! RAM:0x44 == 2 && !(RAM:0x45 & 0x80)
//! ```
//!
//! Operators, loosest first: `||`, `&&`, comparisons (`== != < <= > >=`), `|`, `&`, `+ -`,
//! and the prefix `!` and `-`. Numbers are decimal or `0x` hex; true is 1, false is 0.

use anyhow::{anyhow, bail, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(i64),
    Mem { region: String, addr: u64, width: u8 },
    Ran { region: String, offset: u64 },
    Frame,
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    BitOr,
    BitAnd,
    Add,
    Sub,
}

/// What evaluating needs from the emulator.
pub trait Env {
    fn byte(&mut self, region: &str, addr: u64) -> Result<u8>;
    fn ran(&mut self, region: &str, offset: u64) -> bool;
    fn frame(&self) -> u64;
}

impl Expr {
    pub fn parse(s: &str) -> Result<Expr> {
        let toks = lex(s)?;
        let mut p = Parser { t: toks, i: 0 };
        let e = p.or()?;
        if p.i < p.t.len() {
            bail!("unexpected `{}` in the condition", p.t[p.i]);
        }
        Ok(e)
    }

    pub fn eval(&self, env: &mut dyn Env) -> Result<i64> {
        Ok(match self {
            Expr::Num(n) => *n,
            Expr::Mem { region, addr, width } => {
                let mut v = 0i64;
                for i in 0..*width as u64 {
                    v |= (env.byte(region, addr + i)? as i64) << (8 * i);
                }
                v
            }
            Expr::Ran { region, offset } => env.ran(region, *offset) as i64,
            Expr::Frame => env.frame() as i64,
            Expr::Not(e) => (e.eval(env)? == 0) as i64,
            Expr::Neg(e) => -e.eval(env)?,
            Expr::Bin(op, a, b) => {
                let x = a.eval(env)?;
                if *op == Op::Or && x != 0 {
                    return Ok(1);
                }
                if *op == Op::And && x == 0 {
                    return Ok(0);
                }
                let y = b.eval(env)?;
                match op {
                    Op::Or | Op::And => (y != 0) as i64,
                    Op::Eq => (x == y) as i64,
                    Op::Ne => (x != y) as i64,
                    Op::Lt => (x < y) as i64,
                    Op::Le => (x <= y) as i64,
                    Op::Gt => (x > y) as i64,
                    Op::Ge => (x >= y) as i64,
                    Op::BitOr => x | y,
                    Op::BitAnd => x & y,
                    Op::Add => x + y,
                    Op::Sub => x - y,
                }
            }
        })
    }

    /// Every memory reference in the expression, to report the values seen.
    pub fn memory_refs(&self, out: &mut Vec<Expr>) {
        match self {
            Expr::Mem { .. } => {
                if !out.contains(self) {
                    out.push(self.clone())
                }
            }
            Expr::Not(e) | Expr::Neg(e) => e.memory_refs(out),
            Expr::Bin(_, a, b) => {
                a.memory_refs(out);
                b.memory_refs(out);
            }
            _ => {}
        }
    }

    /// Every `ran(...)` target.
    pub fn ran_refs(&self, out: &mut Vec<(String, u64)>) {
        match self {
            Expr::Ran { region, offset } => out.push((region.clone(), *offset)),
            Expr::Not(e) | Expr::Neg(e) => e.ran_refs(out),
            Expr::Bin(_, a, b) => {
                a.ran_refs(out);
                b.ran_refs(out);
            }
            _ => {}
        }
    }

    pub fn text(&self) -> String {
        match self {
            Expr::Num(n) => n.to_string(),
            Expr::Mem { region, addr, width: 1 } => format!("{region}:{addr:#x}"),
            Expr::Mem { region, addr, .. } => format!("u16({region}:{addr:#x})"),
            Expr::Ran { region, offset } => format!("ran({region}:{offset:#x})"),
            Expr::Frame => "frame".into(),
            Expr::Not(e) => format!("!{}", e.text()),
            Expr::Neg(e) => format!("-{}", e.text()),
            Expr::Bin(op, a, b) => {
                let o = match op {
                    Op::Or => "||",
                    Op::And => "&&",
                    Op::Eq => "==",
                    Op::Ne => "!=",
                    Op::Lt => "<",
                    Op::Le => "<=",
                    Op::Gt => ">",
                    Op::Ge => ">=",
                    Op::BitOr => "|",
                    Op::BitAnd => "&",
                    Op::Add => "+",
                    Op::Sub => "-",
                };
                format!("({} {o} {})", a.text(), b.text())
            }
        }
    }

    /// For a comparison against a constant, how far the left side is from making it true
    /// (0 when true): lets a search rank attempts. None when there is no such measure.
    pub fn distance(&self, env: &mut dyn Env) -> Option<i64> {
        match self {
            Expr::Bin(op, a, b) => {
                let (x, y) = (a.eval(env).ok()?, b.eval(env).ok()?);
                Some(match op {
                    Op::Ge => (y - x).max(0),
                    Op::Gt => (y + 1 - x).max(0),
                    Op::Le => (x - y).max(0),
                    Op::Lt => (x + 1 - y).max(0),
                    Op::Eq => (x - y).abs(),
                    Op::And => a.distance(env).unwrap_or(0) + b.distance(env).unwrap_or(0),
                    Op::Or => a.distance(env).unwrap_or(i64::MAX).min(b.distance(env).unwrap_or(i64::MAX)),
                    _ => return None,
                })
            }
            _ => None,
        }
    }
}

fn lex(s: &str) -> Result<Vec<String>> {
    let mut out = vec![];
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        let two: String = c[i..(i + 2).min(c.len())].iter().collect();
        if ["||", "&&", "==", "!=", "<=", ">="].contains(&two.as_str()) {
            out.push(two);
            i += 2;
            continue;
        }
        if "()<>!+-|&,".contains(ch) {
            out.push(ch.to_string());
            i += 1;
            continue;
        }
        if ch.is_alphanumeric() || ch == '_' || ch == '$' {
            let st = i;
            // A word, with REGION:0xADDR kept together.
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$' || c[i] == ':' || c[i] == '.') {
                i += 1;
            }
            out.push(c[st..i].iter().collect());
            continue;
        }
        bail!("unexpected `{ch}` in the condition");
    }
    Ok(out)
}

struct Parser {
    t: Vec<String>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&str> {
        self.t.get(self.i).map(|s| s.as_str())
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.peek() == Some(s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<()> {
        if self.eat(s) {
            Ok(())
        } else {
            bail!("expected `{s}` in the condition")
        }
    }
    fn bin(&mut self, ops: &[(&str, Op)], next: fn(&mut Parser) -> Result<Expr>) -> Result<Expr> {
        let mut e = next(self)?;
        'outer: loop {
            for (t, op) in ops {
                if self.eat(t) {
                    let r = next(self)?;
                    e = Expr::Bin(*op, Box::new(e), Box::new(r));
                    continue 'outer;
                }
            }
            return Ok(e);
        }
    }
    fn or(&mut self) -> Result<Expr> {
        self.bin(&[("||", Op::Or)], Parser::and)
    }
    fn and(&mut self) -> Result<Expr> {
        self.bin(&[("&&", Op::And)], Parser::cmp)
    }
    fn cmp(&mut self) -> Result<Expr> {
        self.bin(&[("==", Op::Eq), ("!=", Op::Ne), ("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt)], Parser::bits)
    }
    fn bits(&mut self) -> Result<Expr> {
        self.bin(&[("|", Op::BitOr), ("&", Op::BitAnd)], Parser::sum)
    }
    fn sum(&mut self) -> Result<Expr> {
        self.bin(&[("+", Op::Add), ("-", Op::Sub)], Parser::unary)
    }
    fn unary(&mut self) -> Result<Expr> {
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.atom()
    }
    fn atom(&mut self) -> Result<Expr> {
        let t = self.peek().ok_or_else(|| anyhow!("the condition ends too soon"))?.to_string();
        self.i += 1;
        if t == "(" {
            let e = self.or()?;
            self.expect(")")?;
            return Ok(e);
        }
        let lower = t.to_ascii_lowercase();
        if lower == "frame" {
            return Ok(Expr::Frame);
        }
        if lower == "ran" || lower == "u16" {
            self.expect("(")?;
            let inner = self.peek().ok_or_else(|| anyhow!("{t}( needs an address"))?.to_string();
            self.i += 1;
            self.expect(")")?;
            let (region, addr) = region_addr(&inner)?;
            return Ok(if lower == "ran" { Expr::Ran { region, offset: addr } } else { Expr::Mem { region, addr, width: 2 } });
        }
        if t.contains(':') {
            let (region, addr) = region_addr(&t)?;
            return Ok(Expr::Mem { region, addr, width: 1 });
        }
        Ok(Expr::Num(num(&t)?))
    }
}

fn num(s: &str) -> Result<i64> {
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).or_else(|| s.strip_prefix('$')) {
        Some(h) => i64::from_str_radix(h, 16),
        None => s.parse(),
    };
    r.map_err(|_| anyhow!("`{s}` is not a number, a memory reference (RAM:0x44) or ran(...)"))
}

fn region_addr(s: &str) -> Result<(String, u64)> {
    let (r, a) = s.split_once(':').ok_or_else(|| anyhow!("expected REGION:0xADDR, got {s}"))?;
    let a = a.trim();
    let h = a.strip_prefix("0x").or_else(|| a.strip_prefix("0X")).or_else(|| a.strip_prefix('$')).unwrap_or(a);
    Ok((r.trim().to_ascii_uppercase(), u64::from_str_radix(h, 16).map_err(|_| anyhow!("not a hex address: {a}"))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct E(Vec<u8>, u64);
    impl Env for E {
        fn byte(&mut self, _r: &str, a: u64) -> Result<u8> {
            Ok(self.0[a as usize])
        }
        fn ran(&mut self, _r: &str, o: u64) -> bool {
            o == 7
        }
        fn frame(&self) -> u64 {
            self.1
        }
    }

    #[test]
    fn evaluates() {
        let mut e = E(vec![0, 2, 0x34, 0x12, 0x80], 10);
        let t = |s: &str, e: &mut E| Expr::parse(s).unwrap().eval(e).unwrap();
        assert_eq!(t("RAM:0x1 >= 2", &mut e), 1);
        assert_eq!(t("RAM:1 == 3 || RAM:1 == 2", &mut e), 1);
        assert_eq!(t("u16(RAM:0x2) == 0x1234", &mut e), 1);
        assert_eq!(t("!(RAM:0x4 & 0x80)", &mut e), 0);
        assert_eq!(t("ran(PRG:0x7) && frame > 5", &mut e), 1);
        assert_eq!(t("RAM:1 + 1 - 3", &mut e), 0);
        assert!(Expr::parse("RAM:0x1 >=").is_err());
        assert_eq!(Expr::parse("RAM:0x1 >= 5").unwrap().distance(&mut e), Some(3));
    }
}
