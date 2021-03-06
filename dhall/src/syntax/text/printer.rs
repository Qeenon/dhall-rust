use crate::builtins::Builtin;
use crate::operations::{BinOp, OpKind};
use crate::syntax::*;
use itertools::Itertools;
use std::fmt::{self, Display};

// There is a one-to-one correspondence between the formatter and the grammar. Each phase is
// named after a corresponding grammar group, and the structure of the formatter reflects
// the relationship between the corresponding grammar rules. This leads to the nice property
// of automatically getting all the parentheses and precedences right (in a manner dual do Pratt
// parsing).
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
enum PrintPhase {
    // `expression`
    Base,
    // `operator-expression`
    Operator,
    // All the `<operator>-expression`s
    BinOp(self::BinOp),
    // `application-expression`
    App,
    // `import-expression`
    Import,
    // `primitive-expression`
    Primitive,
}

// Wraps an Expr with a phase, so that phase selection can be done separate from the actual
// printing.
#[derive(Copy, Clone)]
struct PhasedExpr<'a>(&'a Expr, PrintPhase);

impl<'a> PhasedExpr<'a> {
    fn phase(self, phase: PrintPhase) -> PhasedExpr<'a> {
        PhasedExpr(self.0, phase)
    }
}

impl UnspannedExpr {
    // Annotate subexpressions with the appropriate phase, defaulting to Base
    fn annotate_with_phases(&self) -> ExprKind<PhasedExpr<'_>> {
        use ExprKind::*;
        use OpKind::*;
        use PrintPhase::*;
        let with_base = self.map_ref(|e| PhasedExpr(e, Base));
        match with_base {
            Pi(a, b, c) => {
                if &String::from(&a) == "_" {
                    Pi(a, b.phase(Operator), c)
                } else {
                    Pi(a, b, c)
                }
            }
            Op(Merge(a, b, c)) => Op(Merge(
                a.phase(PrintPhase::Import),
                b.phase(PrintPhase::Import),
                c.map(|x| x.phase(PrintPhase::App)),
            )),
            Op(ToMap(a, b)) => Op(ToMap(
                a.phase(PrintPhase::Import),
                b.map(|x| x.phase(PrintPhase::App)),
            )),
            Annot(a, b) => Annot(a.phase(Operator), b),
            Op(OpKind::BinOp(op, a, b)) => Op(OpKind::BinOp(
                op,
                a.phase(PrintPhase::BinOp(op)),
                b.phase(PrintPhase::BinOp(op)),
            )),
            SomeLit(e) => SomeLit(e.phase(PrintPhase::Import)),
            Op(OpKind::App(f, a)) => Op(OpKind::App(
                f.phase(PrintPhase::App),
                a.phase(PrintPhase::Import),
            )),
            Op(Field(a, b)) => Op(Field(a.phase(Primitive), b)),
            Op(Projection(e, ls)) => Op(Projection(e.phase(Primitive), ls)),
            Op(ProjectionByExpr(a, b)) => {
                Op(ProjectionByExpr(a.phase(Primitive), b))
            }
            Op(Completion(a, b)) => {
                Op(Completion(a.phase(Primitive), b.phase(Primitive)))
            }
            ExprKind::Import(a) => {
                ExprKind::Import(a.map_ref(|x| x.phase(PrintPhase::Import)))
            }
            e => e,
        }
    }

    fn fmt_phase(
        &self,
        f: &mut fmt::Formatter,
        phase: PrintPhase,
    ) -> Result<(), fmt::Error> {
        use ExprKind::*;
        use OpKind::*;

        let needs_paren = match self {
            Lam(_, _, _)
            | Pi(_, _, _)
            | Let(_, _, _, _)
            | SomeLit(_)
            | EmptyListLit(_)
            | Op(BoolIf(_, _, _))
            | Op(Merge(_, _, _))
            | Op(ToMap(_, _))
            | Annot(_, _) => phase > PrintPhase::Base,
            // Precedence is magically handled by the ordering of BinOps. This is reverse Pratt
            // parsing.
            Op(BinOp(op, _, _)) => phase > PrintPhase::BinOp(*op),
            Op(App(_, _)) => phase > PrintPhase::App,
            Op(Completion(_, _)) => phase > PrintPhase::Import,
            _ => false,
        };

        if needs_paren {
            f.write_str("(")?;
        }
        self.annotate_with_phases().fmt(f)?;
        if needs_paren {
            f.write_str(")")?;
        }

        Ok(())
    }
}

fn fmt_list<T, I, F>(
    open: &str,
    sep: &str,
    close: &str,
    it: I,
    f: &mut fmt::Formatter,
    func: F,
) -> Result<(), fmt::Error>
where
    I: IntoIterator<Item = T>,
    F: Fn(T, &mut fmt::Formatter) -> Result<(), fmt::Error>,
{
    f.write_str(open)?;
    for (i, x) in it.into_iter().enumerate() {
        if i > 0 {
            f.write_str(sep)?;
        }
        func(x, f)?;
    }
    f.write_str(close)
}

fn fmt_label(label: &Label, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    // TODO: distinguish between reserved and nonreserved locations for quoting builtins
    let s = String::from(label);
    let is_reserved = match s.as_str() {
        "let" | "in" | "if" | "then" | "else" | "Type" | "Kind" | "Sort"
        | "True" | "False" | "Some" => true,
        _ => Builtin::parse(&s).is_some(),
    };
    if s.is_empty() {
        write!(f, "``")
    } else if !is_reserved
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        write!(f, "{}", s)
    } else {
        write!(f, "`{}`", s)
    }
}

/// Generic instance that delegates to subexpressions
impl<SE: Display + Clone> Display for ExprKind<SE> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use crate::syntax::ExprKind::*;
        match self {
            Var(a) => a.fmt(f)?,
            Lam(a, b, c) => {
                write!(f, "λ(")?;
                fmt_label(a, f)?;
                write!(f, " : {}) → {}", b, c)?;
            }
            Pi(a, b, c) if &String::from(a) == "_" => {
                write!(f, "{} → {}", b, c)?;
            }
            Pi(a, b, c) => {
                write!(f, "∀(")?;
                fmt_label(a, f)?;
                write!(f, " : {}) → {}", b, c)?;
            }
            Let(a, b, c, d) => {
                write!(f, "let ")?;
                fmt_label(a, f)?;
                if let Some(b) = b {
                    write!(f, " : {}", b)?;
                }
                write!(f, " = {} in {}", c, d)?;
            }
            Const(k) => k.fmt(f)?,
            Builtin(v) => v.fmt(f)?,
            Num(a) => a.fmt(f)?,
            TextLit(a) => a.fmt(f)?,
            SomeLit(e) => {
                write!(f, "Some {}", e)?;
            }
            EmptyListLit(t) => {
                write!(f, "[] : {}", t)?;
            }
            NEListLit(es) => {
                fmt_list("[", ", ", "]", es, f, Display::fmt)?;
            }
            RecordLit(a) if a.is_empty() => f.write_str("{=}")?,
            RecordLit(a) => fmt_list("{ ", ", ", " }", a, f, |(k, v), f| {
                fmt_label(k, f)?;
                write!(f, " = {}", v)
            })?,
            RecordType(a) if a.is_empty() => f.write_str("{}")?,
            RecordType(a) => fmt_list("{ ", ", ", " }", a, f, |(k, t), f| {
                fmt_label(k, f)?;
                write!(f, " : {}", t)
            })?,
            UnionType(a) => fmt_list("< ", " | ", " >", a, f, |(k, v), f| {
                fmt_label(k, f)?;
                if let Some(v) = v {
                    write!(f, ": {}", v)?;
                }
                Ok(())
            })?,
            Op(op) => {
                op.fmt(f)?;
            }
            Annot(a, b) => {
                write!(f, "{} : {}", a, b)?;
            }
            Assert(a) => {
                write!(f, "assert : {}", a)?;
            }
            Import(a) => a.fmt(f)?,
        }
        Ok(())
    }
}

/// Generic instance that delegates to subexpressions
impl<SE: Display + Clone> Display for OpKind<SE> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use OpKind::*;
        match self {
            App(a, b) => {
                write!(f, "{} {}", a, b)?;
            }
            BinOp(op, a, b) => {
                write!(f, "{} {} {}", a, op, b)?;
            }
            BoolIf(a, b, c) => {
                write!(f, "if {} then {} else {}", a, b, c)?;
            }
            Merge(a, b, c) => {
                write!(f, "merge {} {}", a, b)?;
                if let Some(c) = c {
                    write!(f, " : {}", c)?;
                }
            }
            ToMap(a, b) => {
                write!(f, "toMap {}", a)?;
                if let Some(b) = b {
                    write!(f, " : {}", b)?;
                }
            }
            Field(a, b) => {
                write!(f, "{}.", a)?;
                fmt_label(b, f)?;
            }
            Projection(e, ls) => {
                write!(f, "{}.", e)?;
                fmt_list("{ ", ", ", " }", ls, f, fmt_label)?;
            }
            ProjectionByExpr(a, b) => {
                write!(f, "{}.({})", a, b)?;
            }
            Completion(a, b) => {
                write!(f, "{}::{}", a, b)?;
            }
            With(a, ls, b) => {
                let ls = ls.iter().join(".");
                write!(f, "{} with {} = {}", a, ls, b)?;
            }
        }
        Ok(())
    }
}

impl Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.kind().fmt_phase(f, PrintPhase::Base)
    }
}

impl Display for NumKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use NumKind::*;
        match self {
            Bool(true) => f.write_str("True")?,
            Bool(false) => f.write_str("False")?,
            Natural(a) => a.fmt(f)?,
            Integer(a) if *a >= 0 => {
                f.write_str("+")?;
                a.fmt(f)?;
            }
            Integer(a) => a.fmt(f)?,
            Double(a) => a.fmt(f)?,
        }
        Ok(())
    }
}

impl<'a> Display for PhasedExpr<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.0.as_ref().fmt_phase(f, self.1)
    }
}

impl<SubExpr: Display> Display for InterpolatedText<SubExpr> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str("\"")?;
        for x in self.iter() {
            match x {
                InterpolatedTextContents::Text(a) => {
                    for c in a.chars() {
                        match c {
                            '\\' => f.write_str("\\\\"),
                            '"' => f.write_str("\\\""),
                            '$' => f.write_str("\\u0024"),
                            '\u{0008}' => f.write_str("\\b"),
                            '\u{000C}' => f.write_str("\\f"),
                            '\n' => f.write_str("\\n"),
                            '\r' => f.write_str("\\r"),
                            '\t' => f.write_str("\\t"),
                            '\u{0000}'..='\u{001F}' => {
                                // Escape to an explicit "\u{XXXX}" form
                                let escaped: String =
                                    c.escape_default().collect();
                                // Print as "\uXXXX"
                                write!(
                                    f,
                                    "\\u{:0>4}",
                                    &escaped[3..escaped.len() - 1]
                                )
                            }
                            c => write!(f, "{}", c),
                        }?;
                    }
                }
                InterpolatedTextContents::Expr(e) => {
                    f.write_str("${ ")?;
                    e.fmt(f)?;
                    f.write_str(" }")?;
                }
            }
        }
        f.write_str("\"")?;
        Ok(())
    }
}

impl Display for Const {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

impl Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use BinOp::*;
        f.write_str(match self {
            BoolOr => "||",
            TextAppend => "++",
            NaturalPlus => "+",
            BoolAnd => "&&",
            RecursiveRecordMerge => "∧",
            NaturalTimes => "*",
            BoolEQ => "==",
            BoolNE => "!=",
            RecursiveRecordTypeMerge => "⩓",
            ImportAlt => "?",
            RightBiasedRecordMerge => "⫽",
            ListAppend => "#",
            Equivalence => "≡",
        })
    }
}

impl Display for NaiveDouble {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let v = f64::from(*self);
        if v == std::f64::INFINITY {
            f.write_str("Infinity")
        } else if v == std::f64::NEG_INFINITY {
            f.write_str("-Infinity")
        } else if v.is_nan() {
            f.write_str("NaN")
        } else if v == 0.0 && v.is_sign_negative() {
            f.write_str("-0.0")
        } else {
            let s = format!("{}", v);
            if s.contains('e') || s.contains('.') {
                f.write_str(&s)
            } else {
                write!(f, "{}.0", s)
            }
        }
    }
}

impl Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", String::from(self))
    }
}

impl Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self {
            Hash::SHA256(hash) => write!(f, "sha256:{}", hex::encode(hash)),
        }
    }
}

impl<SubExpr: Display> Display for Import<SubExpr> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use FilePrefix::*;
        use ImportMode::*;
        use ImportTarget::*;
        let quote_if_needed = |s: &str| -> String {
            if s.chars().all(|c| c.is_ascii_alphanumeric()) {
                s.to_string()
            } else {
                format!("\"{}\"", s)
            }
        };

        match &self.location {
            Local(prefix, path) => {
                let prefix = match prefix {
                    Here => ".",
                    Parent => "..",
                    Home => "~",
                    Absolute => "",
                };
                write!(f, "{}/", prefix)?;
                let path: String = path
                    .file_path
                    .iter()
                    .map(|c| quote_if_needed(&*c))
                    .join("/");
                f.write_str(&path)?;
            }
            Remote(url) => {
                write!(f, "{}://{}/", url.scheme, url.authority)?;
                let path: String = url.path.file_path.iter().join("/");
                f.write_str(&path)?;
                if let Some(q) = &url.query {
                    write!(f, "?{}", q)?
                }
                if let Some(h) = &url.headers {
                    write!(f, " using {}", h)?
                }
            }
            Env(s) => {
                write!(f, "env:")?;
                if s.chars().all(|c| c.is_ascii_alphanumeric()) {
                    write!(f, "{}", s)?;
                } else {
                    write!(f, "\"")?;
                    for c in s.chars() {
                        match c {
                            '"' => f.write_str("\\\"")?,
                            '\\' => f.write_str("\\\\")?,
                            '\u{0007}' => f.write_str("\\a")?,
                            '\u{0008}' => f.write_str("\\b")?,
                            '\u{000C}' => f.write_str("\\f")?,
                            '\n' => f.write_str("\\n")?,
                            '\r' => f.write_str("\\r")?,
                            '\t' => f.write_str("\\t")?,
                            '\u{000B}' => f.write_str("\\v")?,
                            _ => write!(f, "{}", c)?,
                        }
                    }
                    write!(f, "\"")?;
                }
            }
            Missing => {
                write!(f, "missing")?;
            }
        }
        if let Some(hash) = &self.hash {
            write!(f, " ")?;
            hash.fmt(f)?;
        }
        match self.mode {
            Code => {}
            RawText => write!(f, " as Text")?,
            Location => write!(f, " as Location")?,
        }
        Ok(())
    }
}

impl Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use crate::syntax::Scheme::*;
        f.write_str(match *self {
            HTTP => "http",
            HTTPS => "https",
        })
    }
}

impl Display for V {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let V(x, n) = self;
        fmt_label(x, f)?;
        if *n != 0 {
            write!(f, "@{}", n)?;
        }
        Ok(())
    }
}
