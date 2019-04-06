#![allow(non_snake_case)]
use std::collections::HashSet;
use std::fmt;

use crate::expr::*;
use dhall_core;
use dhall_core::context::Context;
use dhall_core::*;
use dhall_generator as dhall;

use self::TypeMessage::*;

impl Resolved {
    pub fn typecheck(self) -> Result<Typed, TypeError<X>> {
        // let typ = Type(Box::new(Normalized(crate::typecheck::type_of(
        //     self.0.clone(),
        // )?)));
        // Ok(Typed(self.0, typ))
        let typ = crate::typecheck::type_of(self.0.clone())?;
        Ok(Typed(self.0, typ))
    }
}
impl Typed {
    pub fn get_type(&self) -> &Type {
        &self.1
    }
}
impl Normalized {
    pub fn get_type(&self) -> &Type {
        &self.1
    }
}

fn axiom<S>(c: Const) -> Result<Const, TypeError<S>> {
    use dhall_core::Const::*;
    use dhall_core::ExprF::*;
    match c {
        Type => Ok(Kind),
        Kind => Err(TypeError::new(&Context::new(), rc(Const(Kind)), Untyped)),
    }
}

fn rule(a: Const, b: Const) -> Result<Const, ()> {
    use dhall_core::Const::*;
    match (a, b) {
        (Type, Kind) => Err(()),
        (Kind, Kind) => Ok(Kind),
        (Type, Type) | (Kind, Type) => Ok(Type),
    }
}

fn match_vars(vl: &V<Label>, vr: &V<Label>, ctx: &[(Label, Label)]) -> bool {
    let mut vl = vl.clone();
    let mut vr = vr.clone();
    let mut ctx = ctx.to_vec();
    ctx.reverse();
    while let Some((xL2, xR2)) = &ctx.pop() {
        match (&vl, &vr) {
            (V(xL, 0), V(xR, 0)) if xL == xL2 && xR == xR2 => return true,
            (V(xL, nL), V(xR, nR)) => {
                let nL2 = if xL == xL2 { nL - 1 } else { *nL };
                let nR2 = if xR == xR2 { nR - 1 } else { *nR };
                vl = V(xL.clone(), nL2);
                vr = V(xR.clone(), nR2);
            }
        }
    }
    vl == vr
}

// Takes normalized expressions as input
fn prop_equal<S, T>(eL0: &Expr<S, X>, eR0: &Expr<T, X>) -> bool
where
    S: ::std::fmt::Debug,
    T: ::std::fmt::Debug,
{
    use dhall_core::ExprF::*;
    fn go<S, T>(
        ctx: &mut Vec<(Label, Label)>,
        el: &Expr<S, X>,
        er: &Expr<T, X>,
    ) -> bool
    where
        S: ::std::fmt::Debug,
        T: ::std::fmt::Debug,
    {
        match (el, er) {
            (&Const(a), &Const(b)) => a == b,
            (&Builtin(a), &Builtin(b)) => a == b,
            (&Var(ref vL), &Var(ref vR)) => match_vars(vL, vR, ctx),
            (&Pi(ref xL, ref tL, ref bL), &Pi(ref xR, ref tR, ref bR)) => {
                //ctx <- State.get
                let eq1 = go(ctx, tL.as_ref(), tR.as_ref());
                if eq1 {
                    //State.put ((xL, xR):ctx)
                    ctx.push((xL.clone(), xR.clone()));
                    let eq2 = go(ctx, bL.as_ref(), bR.as_ref());
                    //State.put ctx
                    let _ = ctx.pop();
                    eq2
                } else {
                    false
                }
            }
            (&App(ref fL, ref aL), &App(ref fR, ref aR)) => {
                go(ctx, fL.as_ref(), fR.as_ref())
                    && aL.len() == aR.len()
                    && aL
                        .iter()
                        .zip(aR.iter())
                        .all(|(aL, aR)| go(ctx, aL.as_ref(), aR.as_ref()))
            }
            (&RecordType(ref ktsL0), &RecordType(ref ktsR0)) => {
                ktsL0.len() == ktsR0.len()
                    && ktsL0.iter().zip(ktsR0.iter()).all(
                        |((kL, tL), (kR, tR))| {
                            kL == kR && go(ctx, tL.as_ref(), tR.as_ref())
                        },
                    )
            }
            (&UnionType(ref ktsL0), &UnionType(ref ktsR0)) => {
                ktsL0.len() == ktsR0.len()
                    && ktsL0.iter().zip(ktsR0.iter()).all(
                        |((kL, tL), (kR, tR))| {
                            kL == kR && go(ctx, tL.as_ref(), tR.as_ref())
                        },
                    )
            }
            (_, _) => false,
        }
    }
    let mut ctx = vec![];
    go::<S, T>(&mut ctx, eL0, eR0)
}

fn type_of_builtin<S>(b: Builtin) -> Expr<S, X> {
    use dhall_core::Builtin::*;
    match b {
        Bool | Natural | Integer | Double | Text => dhall::expr!(Type),
        List | Optional => dhall::expr!(
            Type -> Type
        ),
        NaturalFold => dhall::expr!(
            Natural ->
            forall (natural: Type) ->
            forall (succ: natural -> natural) ->
            forall (zero: natural) ->
            natural
        ),
        NaturalBuild => dhall::expr!(
            (forall (natural: Type) ->
                forall (succ: natural -> natural) ->
                forall (zero: natural) ->
                natural) ->
            Natural
        ),
        NaturalIsZero | NaturalEven | NaturalOdd => dhall::expr!(
            Natural -> Bool
        ),
        ListBuild => dhall::expr!(
            forall (a: Type) ->
            (forall (list: Type) ->
                forall (cons: a -> list -> list) ->
                forall (nil: list) ->
                list) ->
            List a
        ),
        ListFold => dhall::expr!(
            forall (a: Type) ->
            List a ->
            forall (list: Type) ->
            forall (cons: a -> list -> list) ->
            forall (nil: list) ->
            list
        ),
        ListLength => dhall::expr!(forall (a: Type) -> List a -> Natural),
        ListHead | ListLast => {
            dhall::expr!(forall (a: Type) -> List a -> Optional a)
        }
        ListIndexed => dhall::expr!(
            forall (a: Type) ->
            List a ->
            List { index: Natural, value: a }
        ),
        ListReverse => dhall::expr!(
            forall (a: Type) -> List a -> List a
        ),
        OptionalFold => dhall::expr!(
            forall (a: Type) ->
            Optional a ->
            forall (optional: Type) ->
            forall (just: a -> optional) ->
            forall (nothing: optional) ->
            optional
        ),
        _ => panic!("Unimplemented typecheck case: {:?}", b),
    }
}

fn ensure_equal<'a, S, F1, F2>(
    x: &'a Expr<S, X>,
    y: &'a Expr<S, X>,
    mkerr: F1,
    mkmsg: F2,
) -> Result<(), TypeError<S>>
where
    S: std::fmt::Debug,
    F1: FnOnce(TypeMessage<S>) -> TypeError<S>,
    F2: FnOnce() -> TypeMessage<S>,
{
    if prop_equal(x, y) {
        Ok(())
    } else {
        Err(mkerr(mkmsg()))
    }
}

/// Type-check an expression and return the expression's type if type-checking
/// succeeds or an error if type-checking fails
///
/// `type_with` normalizes the type since while type-checking. It expects the
/// context to contain only normalized expressions.
pub fn type_with(
    ctx: &Context<Label, SubExpr<X, X>>,
    e: SubExpr<X, X>,
) -> Result<Typed, TypeError<X>> {
    use dhall_core::BinOp::*;
    use dhall_core::Builtin::*;
    use dhall_core::Const::*;
    use dhall_core::ExprF::*;
    let mkerr = |msg: TypeMessage<_>| TypeError::new(ctx, e.clone(), msg);
    let ensure_const = |x: &SubExpr<_, _>, msg: TypeMessage<_>| match x.as_ref()
    {
        Const(k) => Ok(*k),
        _ => Err(mkerr(msg)),
    };
    let ensure_is_type =
        |x: SubExpr<_, _>, msg: TypeMessage<_>| match x.as_ref() {
            Const(Type) => Ok(()),
            _ => Err(mkerr(msg)),
        };

    enum Ret {
        ErrRet(TypeError<X>),
        OkNormalized(Normalized),
        OkRet(Expr<X, X>),
    }
    use Ret::*;
    let ret = match e.as_ref() {
        Lam(x, t, b) => {
            let t2 = type_with(ctx, t.clone())?.normalize();
            let ctx2 = ctx
                .insert(x.clone(), t2.0.clone())
                .map(|e| shift(1, &V(x.clone(), 0), e));
            let tB = type_with(&ctx2, b.clone())?.1;
            let _ = type_with(ctx, rc(Pi(x.clone(), t.clone(), tB.clone())))?.1;
            OkRet(Pi(x.clone(), t2.0, tB))
        }
        Pi(x, tA, tB) => {
            let tA = type_with(ctx, tA.clone())?.normalize();
            let kA =
                ensure_const(tA.get_type(), InvalidInputType(tA.0.clone()))?;

            let ctx2 = ctx
                .insert(x.clone(), tA.0.clone())
                .map(|e| shift(1, &V(x.clone(), 0), e));
            let tB = type_with(&ctx2, tB.clone())?.1;
            let kB = match tB.as_ref() {
                Const(k) => *k,
                _ => {
                    return Err(TypeError::new(
                        &ctx2,
                        e.clone(),
                        InvalidOutputType(tB),
                    ));
                }
            };

            match rule(kA, kB) {
                Err(()) => ErrRet(mkerr(NoDependentTypes(tA.0.clone(), tB))),
                Ok(_) => OkRet(Const(kB)),
            }
        }
        Let(f, mt, r, b) => {
            let r = if let Some(t) = mt {
                rc(Annot(SubExpr::clone(r), SubExpr::clone(t)))
            } else {
                SubExpr::clone(r)
            };

            let r = type_with(ctx, r)?;
            let tr = type_with(ctx, r.get_type().clone())?;
            // Don't bother to provide a `let`-specific version of this error
            // message because this should never happen anyway
            let kR = ensure_const(
                tr.get_type(),
                InvalidInputType(r.get_type().clone()),
            )?;

            let ctx2 = ctx.insert(f.clone(), r.get_type().clone());
            let b = type_with(&ctx2, b.clone())?;
            let tb = type_with(ctx, b.get_type().clone())?;
            // Don't bother to provide a `let`-specific version of this error
            // message because this should never happen anyway
            let kB = ensure_const(
                tb.get_type(),
                InvalidOutputType(b.get_type().clone()),
            )?;

            if let Err(()) = rule(kR, kB) {
                return Err(mkerr(NoDependentLet(
                    r.get_type().clone(),
                    b.get_type().clone(),
                )));
            }

            OkRet(b.get_type().unroll())
        }
        _ => match e
            .as_ref()
            .traverse_ref_simple(|e| type_with(ctx, e.clone()))?
        {
            Lam(_, _, _) => unreachable!(),
            Pi(_, _, _) => unreachable!(),
            Let(_, _, _, _) => unreachable!(),
            Const(c) => OkRet(Const(axiom(c)?)),
            Var(V(x, n)) => match ctx.lookup(&x, n) {
                Some(e) => OkRet(e.unroll()),
                None => ErrRet(mkerr(UnboundVariable)),
            },
            App(f, args) => {
                let mut iter = args.into_iter();
                let mut seen_args: Vec<SubExpr<_, _>> = vec![];
                let mut tf = type_with(ctx, f.get_type().clone())?.normalize();
                while let Some(Typed(a, ta)) = iter.next() {
                    seen_args.push(a.clone());
                    let (x, tx, tb) = match tf.0.as_ref() {
                        Pi(x, tx, tb) => (x, tx, tb),
                        _ => {
                            return Err(mkerr(NotAFunction(
                                rc(App(f.0.clone(), seen_args)),
                                tf,
                            )));
                        }
                    };
                    ensure_equal(tx.as_ref(), ta.as_ref(), mkerr, || {
                        TypeMismatch(
                            rc(App(f.0.clone(), seen_args.clone())),
                            tx.clone(),
                            a.clone(),
                            ta.clone(),
                        )
                    })?;
                    tf =
                        type_with(ctx, subst_shift(&V(x.clone(), 0), &a, &tb))?
                            .normalize();
                }
                OkNormalized(tf)
            }
            Annot(x, t) => {
                let t = t.normalize();
                ensure_equal(
                    t.0.as_ref(),
                    x.get_type().as_ref(),
                    mkerr,
                    || AnnotMismatch(x.clone(), t.clone()),
                )?;
                OkNormalized(t)
            }
            BoolIf(x, y, z) => {
                ensure_equal(
                    x.get_type().as_ref(),
                    &Builtin(Bool),
                    mkerr,
                    || InvalidPredicate(x.clone()),
                )?;
                let ty = type_with(ctx, y.get_type().clone())?.normalize();
                ensure_is_type(
                    ty.get_type().clone(),
                    IfBranchMustBeTerm(true, y.clone()),
                )?;

                let tz = type_with(ctx, z.get_type().clone())?.normalize();
                ensure_is_type(
                    tz.get_type().clone(),
                    IfBranchMustBeTerm(false, z.clone()),
                )?;

                ensure_equal(
                    y.get_type().as_ref(),
                    z.get_type().as_ref(),
                    mkerr,
                    || IfBranchMismatch(y.clone(), z.clone()),
                )?;

                OkNormalized(ty)
            }
            EmptyListLit(t) => {
                ensure_is_type(
                    t.get_type().clone(),
                    InvalidListType(t.0.clone()),
                )?;
                let t = t.normalize().0;
                OkRet(dhall::expr!(List t))
            }
            NEListLit(xs) => {
                let mut iter = xs.into_iter().enumerate();
                let (_, Typed(_, t)) = iter.next().unwrap();
                let s = type_with(ctx, t.clone())?.1;
                ensure_is_type(s, InvalidListType(t.clone()))?;
                for (i, Typed(y, ty)) in iter {
                    ensure_equal(t.as_ref(), ty.as_ref(), mkerr, || {
                        InvalidListElement(i, t.clone(), y.clone(), ty.clone())
                    })?;
                }
                OkRet(dhall::expr!(List t))
            }
            EmptyOptionalLit(t) => {
                ensure_is_type(
                    t.get_type().clone(),
                    InvalidOptionalType(t.0.clone()),
                )?;
                let t = t.normalize().0;
                OkRet(dhall::expr!(Optional t))
            }
            NEOptionalLit(Typed(_, t)) => {
                let s = type_with(ctx, t.clone())?.1;
                ensure_is_type(s, InvalidOptionalType(t.clone()))?;
                OkRet(dhall::expr!(Optional t))
            }
            RecordType(kts) => {
                for (k, Typed(t, tt)) in kts {
                    ensure_is_type(tt, InvalidFieldType(k.clone(), t.clone()))?;
                }
                OkRet(Const(Type))
            }
            RecordLit(kvs) => {
                let kts = kvs
                    .into_iter()
                    .map(|(k, Typed(v, t))| {
                        let s = type_with(ctx, t.clone())?.1;
                        ensure_is_type(s, InvalidField(k.clone(), v.clone()))?;
                        Ok((k.clone(), t))
                    })
                    .collect::<Result<_, _>>()?;
                OkRet(RecordType(kts))
            }
            Field(r, x) => match r.get_type().as_ref() {
                RecordType(kts) => match kts.get(&x) {
                    Some(e) => OkRet(e.unroll()),
                    None => ErrRet(mkerr(MissingField(x.clone(), r.clone()))),
                },
                _ => ErrRet(mkerr(NotARecord(x.clone(), r.clone()))),
            },
            Builtin(b) => OkRet(type_of_builtin(b)),
            BoolLit(_) => OkRet(Builtin(Bool)),
            NaturalLit(_) => OkRet(Builtin(Natural)),
            IntegerLit(_) => OkRet(Builtin(Integer)),
            DoubleLit(_) => OkRet(Builtin(Double)),
            // TODO: check type of interpolations
            TextLit(_) => OkRet(Builtin(Text)),
            BinOp(o, l, r) => {
                let t = Builtin(match o {
                    BoolAnd => Bool,
                    BoolOr => Bool,
                    BoolEQ => Bool,
                    BoolNE => Bool,
                    NaturalPlus => Natural,
                    NaturalTimes => Natural,
                    TextAppend => Text,
                    _ => panic!("Unimplemented typecheck case: {:?}", e),
                });

                ensure_equal(l.get_type().as_ref(), &t, mkerr, || {
                    BinOpTypeMismatch(o, l.clone())
                })?;

                ensure_equal(r.get_type().as_ref(), &t, mkerr, || {
                    BinOpTypeMismatch(o, r.clone())
                })?;

                OkRet(t)
            }
            Embed(p) => match p {},
            _ => panic!("Unimplemented typecheck case: {:?}", e),
        },
    };
    match ret {
        OkRet(ret) => Ok(Typed(e, rc(ret))),
        OkNormalized(ret) => Ok(Typed(e, ret.0)),
        ErrRet(e) => Err(e),
    }
}

/// `typeOf` is the same as `type_with` with an empty context, meaning that the
/// expression must be closed (i.e. no free variables), otherwise type-checking
/// will fail.
pub fn type_of(e: SubExpr<X, X>) -> Result<SubExpr<X, X>, TypeError<X>> {
    let ctx = Context::new();
    type_with(&ctx, e).map(|e| e.1)
}

/// The specific type error
#[derive(Debug)]
pub enum TypeMessage<S> {
    UnboundVariable,
    InvalidInputType(SubExpr<S, X>),
    InvalidOutputType(SubExpr<S, X>),
    NotAFunction(SubExpr<S, X>, Normalized),
    TypeMismatch(SubExpr<S, X>, SubExpr<S, X>, SubExpr<S, X>, SubExpr<S, X>),
    AnnotMismatch(Typed, Normalized),
    Untyped,
    InvalidListElement(usize, SubExpr<S, X>, SubExpr<S, X>, SubExpr<S, X>),
    InvalidListType(SubExpr<S, X>),
    InvalidOptionalElement(SubExpr<S, X>, SubExpr<S, X>, SubExpr<S, X>),
    InvalidOptionalLiteral(usize),
    InvalidOptionalType(SubExpr<S, X>),
    InvalidPredicate(Typed),
    IfBranchMismatch(Typed, Typed),
    IfBranchMustBeTerm(bool, Typed),
    InvalidField(Label, SubExpr<S, X>),
    InvalidFieldType(Label, SubExpr<S, X>),
    InvalidAlternative(Label, SubExpr<S, X>),
    InvalidAlternativeType(Label, SubExpr<S, X>),
    DuplicateAlternative(Label),
    MustCombineARecord(SubExpr<S, X>, SubExpr<S, X>),
    FieldCollision(Label),
    MustMergeARecord(SubExpr<S, X>, SubExpr<S, X>),
    MustMergeUnion(SubExpr<S, X>, SubExpr<S, X>),
    UnusedHandler(HashSet<Label>),
    MissingHandler(HashSet<Label>),
    HandlerInputTypeMismatch(Label, SubExpr<S, X>, SubExpr<S, X>),
    HandlerOutputTypeMismatch(Label, SubExpr<S, X>, SubExpr<S, X>),
    HandlerNotAFunction(Label, SubExpr<S, X>),
    NotARecord(Label, Typed),
    MissingField(Label, Typed),
    BinOpTypeMismatch(BinOp, Typed),
    NoDependentLet(SubExpr<S, X>, SubExpr<S, X>),
    NoDependentTypes(SubExpr<S, X>, SubExpr<S, X>),
}

/// A structured type error that includes context
#[derive(Debug)]
pub struct TypeError<S> {
    pub context: Context<Label, SubExpr<S, X>>,
    pub current: SubExpr<S, X>,
    pub type_message: TypeMessage<S>,
}

impl<S> TypeError<S> {
    pub fn new(
        context: &Context<Label, SubExpr<S, X>>,
        current: SubExpr<S, X>,
        type_message: TypeMessage<S>,
    ) -> Self {
        TypeError {
            context: context.clone(),
            current: current,
            type_message,
        }
    }
}

impl<S: fmt::Debug> ::std::error::Error for TypeMessage<S> {
    fn description(&self) -> &str {
        match *self {
            UnboundVariable => "Unbound variable",
            InvalidInputType(_) => "Invalid function input",
            InvalidOutputType(_) => "Invalid function output",
            NotAFunction(_, _) => "Not a function",
            TypeMismatch(_, _, _, _) => "Wrong type of function argument",
            _ => "Unhandled error",
        }
    }
}

impl<S> fmt::Display for TypeMessage<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self {
            UnboundVariable => {
                f.write_str(include_str!("errors/UnboundVariable.txt"))
            }
            TypeMismatch(e0, e1, e2, e3) => {
                let template = include_str!("errors/TypeMismatch.txt");
                let s = template
                    .replace("$txt0", &format!("{}", e0))
                    .replace("$txt1", &format!("{}", e1))
                    .replace("$txt2", &format!("{}", e2))
                    .replace("$txt3", &format!("{}", e3));
                f.write_str(&s)
            }
            _ => f.write_str("Unhandled error message"),
        }
    }
}
