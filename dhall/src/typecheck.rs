#![allow(non_snake_case)]
use std::borrow::Borrow;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use crate::expr::*;
use crate::traits::DynamicType;
use dhall_core;
use dhall_core::context::Context;
use dhall_core::*;
use dhall_generator as dhall;

use self::TypeMessage::*;

impl<'a> Resolved<'a> {
    pub fn typecheck(self) -> Result<Typed<'static>, TypeError> {
        type_of(self.0.unnote())
    }
    pub fn typecheck_with(
        self,
        ty: &Type,
    ) -> Result<Typed<'static>, TypeError> {
        let expr: SubExpr<_, _> = self.0.unnote();
        let ty: SubExpr<_, _> = ty.clone().unnote().embed()?;
        type_of(dhall::subexpr!(expr: ty))
    }
    /// Pretends this expression has been typechecked. Use with care.
    #[allow(dead_code)]
    pub fn skip_typecheck(self) -> Typed<'a> {
        Typed(self.0.unnote(), None, TypecheckContext::new(), PhantomData)
    }
}
impl<'a> Typed<'a> {
    fn get_type_move(self) -> Result<Type<'static>, TypeError> {
        let (expr, ty) = (self.0, self.1);
        ty.ok_or_else(|| {
            TypeError::new(&TypecheckContext::new(), expr, TypeMessage::Untyped)
        })
    }
}
impl<'a> Normalized<'a> {
    // Expose the outermost constructor
    fn unroll_ref(&self) -> &Expr<X, X> {
        self.as_expr().as_ref()
    }
    fn shift0(&self, delta: isize, label: &Label) -> Self {
        Normalized(
            shift0(delta, label, &self.0),
            self.1.as_ref().map(|t| t.shift0(delta, label)),
            self.2,
        )
    }
    fn shift(&self, delta: isize, var: &V<Label>) -> Self {
        Normalized(
            shift(delta, var, &self.0),
            self.1.as_ref().map(|t| t.shift(delta, var)),
            self.2,
        )
    }
    pub(crate) fn into_type(self) -> Result<Type<'a>, TypeError> {
        self.into_type_ctx(&TypecheckContext::new())
    }
    pub(crate) fn into_type_ctx(
        self,
        ctx: &TypecheckContext,
    ) -> Result<Type<'a>, TypeError> {
        Ok(match self.0.as_ref() {
            ExprF::Const(c) => Type(TypeInternal::Const(*c)),
            ExprF::Pi(_, _, _) => {
                type_with(ctx, self.0.embed_absurd())?.normalize_to_type()?
            }
            _ => Type(TypeInternal::Expr(Box::new(self))),
        })
    }
    fn get_type_move(self) -> Result<Type<'static>, TypeError> {
        let (expr, ty) = (self.0, self.1);
        ty.ok_or_else(|| {
            TypeError::new(
                &TypecheckContext::new(),
                expr.embed_absurd(),
                TypeMessage::Untyped,
            )
        })
    }
}
impl Normalized<'static> {
    fn embed<N>(self) -> SubExpr<N, Normalized<'static>> {
        rc(ExprF::Embed(self))
    }
}
impl<'a> Type<'a> {
    pub(crate) fn as_normalized(
        &self,
    ) -> Result<Cow<Normalized<'a>>, TypeError> {
        Ok(Cow::Owned(self.0.clone().into_normalized()?))
        // match &self.0 {
        //     TypeInternal::Expr(e) => Ok(Cow::Borrowed(e)),
        //     TypeInternal::Const(c) => Ok(Cow::Owned(const_to_normalized(*c))),
        //     TypeInternal::SuperType => Err(TypeError::new(
        //         &TypecheckContext::new(),
        //         rc(ExprF::Const(Const::Sort)),
        //         TypeMessage::Untyped,
        //     )),
        // }
    }
    pub(crate) fn into_normalized(self) -> Result<Normalized<'a>, TypeError> {
        self.0.into_normalized()
    }
    // Expose the outermost constructor
    fn unroll_ref(&self) -> Result<Cow<Expr<X, X>>, TypeError> {
        match self.as_normalized()? {
            Cow::Borrowed(e) => Ok(Cow::Borrowed(e.unroll_ref())),
            Cow::Owned(e) => Ok(Cow::Owned(e.into_expr().unroll())),
        }
    }
    fn internal(&self) -> &TypeInternal {
        &self.0
    }
    fn shift0(&self, delta: isize, label: &Label) -> Self {
        Type(self.0.shift0(delta, label))
    }
    fn shift(&self, delta: isize, var: &V<Label>) -> Self {
        Type(self.0.shift(delta, var))
    }

    fn const_sort() -> Self {
        Type(TypeInternal::Const(Const::Sort))
    }
    fn const_kind() -> Self {
        Type(TypeInternal::Const(Const::Kind))
    }
    pub(crate) fn const_type() -> Self {
        Type(TypeInternal::Const(Const::Type))
    }
}
impl Type<'static> {
    fn embed<N>(self) -> Result<SubExpr<N, Normalized<'static>>, TypeError> {
        Ok(self.into_normalized()?.embed())
    }
}

/// A semantic type. This is partially redundant with `dhall_core::Expr`, on purpose. `TypeInternal` should
/// be limited to syntactic expressions: either written by the user or meant to be printed.
/// The rule is the following: we must _not_ construct values of type `Expr` while typechecking,
/// but only construct `TypeInternal`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeInternal<'a> {
    Const(Const),
    Pi(
        TypecheckContext,
        Const,
        Label,
        Box<Type<'static>>,
        Box<Type<'static>>,
    ),
    RecordType(TypecheckContext, Const, BTreeMap<Label, Type<'static>>),
    /// The type of `Sort`
    SuperType,
    /// This must not contain a value captured by one of the variants above.
    Expr(Box<Normalized<'a>>),
}

impl<'a> TypeInternal<'a> {
    pub(crate) fn into_normalized(self) -> Result<Normalized<'a>, TypeError> {
        match self {
            TypeInternal::Expr(e) => Ok(*e),
            TypeInternal::Pi(ctx, c, x, t, e) => Ok(Typed(
                rc(ExprF::Pi(x, t, e)
                    .traverse_ref_simple(|e| e.clone().embed())?),
                Some(const_to_type(c)),
                ctx,
                PhantomData,
            )
            .normalize()),
            TypeInternal::RecordType(ctx, c, kts) => Ok(Typed(
                rc(ExprF::RecordType(kts)
                    .traverse_ref_simple(|e| e.clone().embed())?),
                Some(const_to_type(c)),
                ctx,
                PhantomData,
            )
            .normalize()),
            TypeInternal::Const(c) => Ok(const_to_normalized(c)),
            TypeInternal::SuperType => Err(TypeError::new(
                &TypecheckContext::new(),
                rc(ExprF::Const(Const::Sort)),
                TypeMessage::Untyped,
            )),
        }
    }
    fn shift0(&self, delta: isize, label: &Label) -> Self {
        self.shift(delta, &V(label.clone(), 0))
    }
    fn shift(&self, delta: isize, var: &V<Label>) -> Self {
        use TypeInternal::*;
        match self {
            Expr(e) => Expr(Box::new(e.shift(delta, var))),
            Pi(ctx, c, x, t, e) => Pi(
                ctx.clone(),
                *c,
                x.clone(),
                Box::new(t.shift(delta, var)),
                Box::new(e.shift(delta, &var.shift0(1, x))),
            ),
            RecordType(ctx, c, kts) => RecordType(
                ctx.clone(),
                *c,
                kts.iter()
                    .map(|(k, v)| (k.clone(), v.shift(delta, var)))
                    .collect(),
            ),
            Const(c) => Const(*c),
            SuperType => SuperType,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum TypedOrType {
    Typed(Typed<'static>),
    Type(Type<'static>),
}

impl TypedOrType {
    fn into_typed(self) -> Result<Typed<'static>, TypeError> {
        match self {
            TypedOrType::Typed(e) => Ok(e),
            TypedOrType::Type(t) => Ok(t.into_normalized()?.into()),
        }
    }
    fn normalize_to_type(self) -> Result<Type<'static>, TypeError> {
        match self {
            TypedOrType::Typed(e) => {
                let ctx = e.2.clone();
                Ok(e.normalize().into_type_ctx(&ctx)?)
            }
            TypedOrType::Type(t) => Ok(t),
        }
    }
    fn get_type(&self) -> Result<Cow<'_, Type<'static>>, TypeError> {
        match self {
            TypedOrType::Typed(e) => e.get_type(),
            TypedOrType::Type(t) => t.get_type(),
        }
    }
    fn get_type_move(self) -> Result<Type<'static>, TypeError> {
        match self {
            TypedOrType::Typed(e) => Ok(e.get_type_move()?),
            TypedOrType::Type(t) => Ok(t.get_type()?.into_owned()),
        }
    }
    fn normalize(self) -> Result<Normalized<'static>, TypeError> {
        match self {
            TypedOrType::Typed(e) => Ok(e.normalize()),
            TypedOrType::Type(t) => Ok(t.into_normalized()?),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum EnvItem {
    Type(V<Label>, Type<'static>),
    Value(Normalized<'static>),
}

impl EnvItem {
    fn shift0(&self, delta: isize, x: &Label) -> Self {
        use EnvItem::*;
        match self {
            Type(v, e) => Type(v.shift0(delta, x), e.shift0(delta, x)),
            Value(e) => Value(e.shift0(delta, x)),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TypecheckContext(pub(crate) Context<Label, EnvItem>);

impl TypecheckContext {
    pub(crate) fn new() -> Self {
        TypecheckContext(Context::new())
    }
    pub(crate) fn insert_type(&self, x: &Label, t: Type<'static>) -> Self {
        TypecheckContext(
            self.0.map(|_, e| e.shift0(1, x)).insert(
                x.clone(),
                EnvItem::Type(V(x.clone(), 0), t.shift0(1, x)),
            ),
        )
    }
    pub(crate) fn insert_value(
        &self,
        x: &Label,
        t: Normalized<'static>,
    ) -> Self {
        TypecheckContext(
            self.0.insert(x.clone(), EnvItem::Value(t.shift0(1, x))),
        )
    }
    pub(crate) fn lookup(
        &self,
        var: &V<Label>,
    ) -> Option<Cow<'_, Type<'static>>> {
        let V(x, n) = var;
        match self.0.lookup(x, *n) {
            Some(EnvItem::Type(_, t)) => Some(Cow::Borrowed(&t)),
            Some(EnvItem::Value(t)) => Some(t.get_type()?),
            None => None,
        }
    }
}

impl PartialEq for TypecheckContext {
    fn eq(&self, _: &Self) -> bool {
        // don't count contexts when comparing stuff
        // this is dirty but needed for now
        true
    }
}
impl Eq for TypecheckContext {}

fn function_check(a: Const, b: Const) -> Result<Const, ()> {
    use dhall_core::Const::*;
    match (a, b) {
        (_, Type) => Ok(Type),
        (Kind, Kind) => Ok(Kind),
        (Sort, Sort) => Ok(Sort),
        (Sort, Kind) => Ok(Sort),
        _ => Err(()),
    }
}

fn match_vars(vl: &V<Label>, vr: &V<Label>, ctx: &[(&Label, &Label)]) -> bool {
    let (V(xL, mut nL), V(xR, mut nR)) = (vl, vr);
    for &(xL2, xR2) in ctx {
        match (nL, nR) {
            (0, 0) if xL == xL2 && xR == xR2 => return true,
            (_, _) => {
                if xL == xL2 {
                    nL -= 1;
                }
                if xR == xR2 {
                    nR -= 1;
                }
            }
        }
    }
    xL == xR && nL == nR
}

// Equality up to alpha-equivalence (renaming of bound variables)
fn prop_equal<T, U>(eL0: T, eR0: U) -> bool
where
    T: Borrow<Type<'static>>,
    U: Borrow<Type<'static>>,
{
    use dhall_core::ExprF::*;
    fn go<'a, S, T>(
        ctx: &mut Vec<(&'a Label, &'a Label)>,
        el: &'a SubExpr<S, X>,
        er: &'a SubExpr<T, X>,
    ) -> bool
    where
        S: ::std::fmt::Debug,
        T: ::std::fmt::Debug,
    {
        match (el.as_ref(), er.as_ref()) {
            (Const(a), Const(b)) => a == b,
            (Builtin(a), Builtin(b)) => a == b,
            (Var(vL), Var(vR)) => match_vars(vL, vR, ctx),
            (Pi(xL, tL, bL), Pi(xR, tR, bR)) => {
                go(ctx, tL, tR) && {
                    ctx.push((xL, xR));
                    let eq2 = go(ctx, bL, bR);
                    ctx.pop();
                    eq2
                }
            }
            (App(fL, aL), App(fR, aR)) => go(ctx, fL, fR) && go(ctx, aL, aR),
            (RecordType(ktsL0), RecordType(ktsR0)) => {
                ktsL0.len() == ktsR0.len()
                    && ktsL0
                        .iter()
                        .zip(ktsR0.iter())
                        .all(|((kL, tL), (kR, tR))| kL == kR && go(ctx, tL, tR))
            }
            (UnionType(ktsL0), UnionType(ktsR0)) => {
                ktsL0.len() == ktsR0.len()
                    && ktsL0.iter().zip(ktsR0.iter()).all(
                        |((kL, tL), (kR, tR))| {
                            kL == kR
                                && match (tL, tR) {
                                    (None, None) => true,
                                    (Some(tL), Some(tR)) => go(ctx, tL, tR),
                                    _ => false,
                                }
                        },
                    )
            }
            (_, _) => false,
        }
    }
    match (&eL0.borrow().0, &eR0.borrow().0) {
        (TypeInternal::SuperType, TypeInternal::SuperType) => true,
        (TypeInternal::SuperType, _) => false,
        (_, TypeInternal::SuperType) => false,
        // (TypeInternal::Const(cl), TypeInternal::Const(cr)) => cl == cr,
        // (TypeInternal::Expr(l), TypeInternal::Expr(r)) => {
        _ => {
            let mut ctx = vec![];
            go(
                &mut ctx,
                eL0.borrow().as_normalized().unwrap().as_expr(),
                eR0.borrow().as_normalized().unwrap().as_expr(),
            )
        }
        // _ => false,
    }
}

fn const_to_normalized<'a>(c: Const) -> Normalized<'a> {
    Normalized(
        rc(ExprF::Const(c)),
        Some(Type(match c {
            Const::Type => TypeInternal::Const(Const::Kind),
            Const::Kind => TypeInternal::Const(Const::Sort),
            Const::Sort => TypeInternal::SuperType,
        })),
        PhantomData,
    )
}

fn const_to_type<'a>(c: Const) -> Type<'a> {
    Type(TypeInternal::Const(c))
}

pub(crate) fn type_of_const<'a>(c: Const) -> Type<'a> {
    match c {
        Const::Type => Type::const_kind(),
        Const::Kind => Type::const_sort(),
        Const::Sort => Type(TypeInternal::SuperType),
    }
}

fn type_of_builtin<N, E>(b: Builtin) -> Expr<N, E> {
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
        OptionalNone => dhall::expr!(
            forall (a: Type) -> Optional a
        ),
        _ => panic!("Unimplemented typecheck case: {:?}", b),
    }
}

macro_rules! ensure_equal {
    ($x:expr, $y:expr, $err:expr $(,)*) => {
        if !prop_equal($x, $y) {
            return Err($err);
        }
    };
}

// Ensure the provided type has type `Type`
macro_rules! ensure_simple_type {
    ($x:expr, $err:expr $(,)*) => {{
        let k = ensure_is_const!($x.get_type()?, $err);
        if k != Type {
            return Err($err);
        }
    }};
}

macro_rules! ensure_is_const {
    ($x:expr, $err:expr $(,)*) => {
        match $x.internal() {
            TypeInternal::Const(k) => *k,
            _ => return Err($err),
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeIntermediate {
    Pi(TypecheckContext, Label, Type<'static>, Type<'static>),
    RecordType(TypecheckContext, BTreeMap<Label, Type<'static>>),
}

impl TypeIntermediate {
    fn typecheck(self) -> Result<TypedOrType, TypeError> {
        let mkerr =
            |ctx, msg| Ok(TypeError::new(ctx, self.clone().into_expr()?, msg));
        match &self {
            TypeIntermediate::Pi(ctx, x, ta, tb) => {
                let ctx2 = ctx.insert_type(x, ta.clone());

                let kA = ensure_is_const!(
                    &ta.get_type()?,
                    mkerr(
                        ctx,
                        InvalidInputType(ta.clone().into_normalized()?),
                    )?,
                );

                let kB = ensure_is_const!(
                    &tb.get_type()?,
                    mkerr(
                        &ctx2,
                        InvalidOutputType(
                            tb.clone()
                                .into_normalized()?
                                .get_type_move()?
                                .into_normalized()?
                        ),
                    )?,
                );

                let k = match function_check(kA, kB) {
                    Ok(k) => k,
                    Err(()) => {
                        return Err(mkerr(
                            ctx,
                            NoDependentTypes(
                                ta.clone().into_normalized()?,
                                tb.clone()
                                    .into_normalized()?
                                    .get_type_move()?
                                    .into_normalized()?,
                            ),
                        )?)
                    }
                };

                Ok(TypedOrType::Type(Type(TypeInternal::Pi(
                    ctx.clone(),
                    k,
                    x.clone(),
                    Box::new(ta.clone()),
                    Box::new(tb.clone()),
                ))))
            }
            TypeIntermediate::RecordType(ctx, kts) => {
                // Check that all types are the same const
                let mut k = None;
                for (x, t) in kts {
                    let k2 = ensure_is_const!(
                        t.get_type()?,
                        mkerr(
                            ctx,
                            InvalidFieldType(
                                x.clone(),
                                TypedOrType::Type(t.clone())
                            )
                        )?
                    );
                    match k {
                        None => k = Some(k2),
                        Some(k1) if k1 != k2 => {
                            return Err(mkerr(
                                ctx,
                                InvalidFieldType(
                                    x.clone(),
                                    TypedOrType::Type(t.clone()),
                                ),
                            )?)
                        }
                        Some(_) => {}
                    }
                }
                // An empty record type has type Type
                let k = k.unwrap_or(dhall_core::Const::Type);

                Ok(TypedOrType::Type(Type(TypeInternal::RecordType(
                    ctx.clone(),
                    k,
                    kts.clone(),
                ))))
            }
        }
    }
    fn into_expr(self) -> Result<SubExpr<X, Normalized<'static>>, TypeError> {
        Ok(rc(match self {
            TypeIntermediate::Pi(_, x, t, e) => ExprF::Pi(x, t, e),
            TypeIntermediate::RecordType(_, kts) => ExprF::RecordType(kts),
        }
        .traverse_ref_simple(|e| e.clone().embed())?))
    }
}

/// Takes an expression that is meant to contain a Type
/// and turn it into a type, typechecking it along the way.
fn mktype(
    ctx: &TypecheckContext,
    e: SubExpr<X, Normalized<'static>>,
) -> Result<Type<'static>, TypeError> {
    Ok(type_with(ctx, e)?.normalize_to_type()?)
}

fn into_simple_type<'a>(ctx: &TypecheckContext, e: SubExpr<X, X>) -> Type<'a> {
    SimpleType(e, PhantomData).into_type_ctx(ctx)
}

fn simple_type_from_builtin<'a>(b: Builtin) -> Type<'a> {
    into_simple_type(&TypecheckContext::new(), rc(ExprF::Builtin(b)))
}

/// Intermediary return type
enum Ret {
    /// Returns the contained value as is
    RetTypedOrType(TypedOrType),
    /// Use the contained Type as the type of the input expression
    RetType(Type<'static>),
    /// Returns an expression that must be typechecked and
    /// turned into a Type first.
    RetExpr(Expr<X, Normalized<'static>>),
}

/// Type-check an expression and return the expression alongside its type if type-checking
/// succeeded, or an error if type-checking failed
fn type_with(
    ctx: &TypecheckContext,
    e: SubExpr<X, Normalized<'static>>,
) -> Result<TypedOrType, TypeError> {
    use dhall_core::ExprF::*;

    use Ret::*;
    let ret = match e.as_ref() {
        Lam(x, t, b) => {
            let tx = mktype(ctx, t.clone())?;
            let ctx2 = ctx.insert_type(x, tx.clone());
            let b = type_with(&ctx2, b.clone())?.into_typed()?;
            let tb = b.get_type_move()?;
            Ok(RetType(
                TypeIntermediate::Pi(ctx.clone(), x.clone(), tx, tb)
                    .typecheck()?
                    .normalize_to_type()?,
            ))
        }
        Pi(x, ta, tb) => {
            let ta = mktype(ctx, ta.clone())?;
            let ctx2 = ctx.insert_type(x, ta.clone());
            let tb = mktype(&ctx2, tb.clone())?;
            Ok(RetTypedOrType(
                TypeIntermediate::Pi(ctx.clone(), x.clone(), ta, tb)
                    .typecheck()?,
            ))
        }
        Let(x, t, v, e) => {
            let v = if let Some(t) = t {
                rc(Annot(v.clone(), t.clone()))
            } else {
                v.clone()
            };

            let v = type_with(ctx, v)?.into_typed()?.normalize();
            let e = type_with(&ctx.insert_value(x, v.clone()), e.clone())?
                .into_typed()?;

            Ok(RetType(e.get_type_move()?))
        }
        Embed(p) => Ok(RetTypedOrType(TypedOrType::Typed(p.clone().into()))),
        _ => type_last_layer(
            ctx,
            // Typecheck recursively all subexpressions
            e.as_ref()
                .traverse_ref_simple(|e| Ok(type_with(ctx, e.clone())?))?,
            e.clone(),
        ),
    }?;
    match ret {
        RetExpr(ret) => Ok(TypedOrType::Typed(Typed(
            e,
            Some(mktype(ctx, rc(ret))?),
            ctx.clone(),
            PhantomData,
        ))),
        RetType(typ) => Ok(TypedOrType::Typed(Typed(
            e,
            Some(typ),
            ctx.clone(),
            PhantomData,
        ))),
        RetTypedOrType(tt) => Ok(tt),
    }
}

/// When all sub-expressions have been typed, check the remaining toplevel
/// layer.
fn type_last_layer(
    ctx: &TypecheckContext,
    e: ExprF<TypedOrType, Label, X, Normalized<'static>>,
    original_e: SubExpr<X, Normalized<'static>>,
) -> Result<Ret, TypeError> {
    use dhall_core::BinOp::*;
    use dhall_core::Builtin::*;
    use dhall_core::Const::*;
    use dhall_core::ExprF::*;
    let mkerr = |msg: TypeMessage<'static>| {
        TypeError::new(ctx, original_e.clone(), msg)
    };

    use Ret::*;
    match e {
        Lam(_, _, _) => unreachable!(),
        Pi(_, _, _) => unreachable!(),
        Let(_, _, _, _) => unreachable!(),
        Embed(_) => unreachable!(),
        Var(var) => match ctx.lookup(&var) {
            Some(e) => Ok(RetType(e.into_owned())),
            None => Err(mkerr(UnboundVariable(var.clone()))),
        },
        App(f, a) => {
            let tf = f.get_type()?.into_owned();
            let (x, tx, tb) = match tf.0 {
                TypeInternal::Pi(_, _, x, tx, tb) => (x, tx, tb),
                _ => return Err(mkerr(NotAFunction(f))),
            };
            ensure_equal!(a.get_type()?, tx.as_ref(), {
                mkerr(TypeMismatch(f, tx.into_normalized()?, a))
            });
            Ok(RetExpr(Let(
                x.clone(),
                None,
                a.normalize()?.embed(),
                tb.into_normalized()?.into_expr().embed_absurd(),
            )))
            // Ok(RetType(mktype(
            //     &ctx.insert_value(&x, a),
            //     tb.into_normalized()?.into_expr().embed_absurd(),
            // )?))
        }
        Annot(x, t) => {
            let t = t.normalize_to_type()?;
            ensure_equal!(
                &t,
                x.get_type()?,
                mkerr(AnnotMismatch(x, t.into_normalized()?))
            );
            Ok(RetType(x.get_type_move()?))
        }
        BoolIf(x, y, z) => {
            ensure_equal!(
                x.get_type()?,
                &simple_type_from_builtin(Bool),
                mkerr(InvalidPredicate(x)),
            );

            ensure_simple_type!(
                y.get_type()?,
                mkerr(IfBranchMustBeTerm(true, y)),
            );

            ensure_simple_type!(
                z.get_type()?,
                mkerr(IfBranchMustBeTerm(false, z)),
            );

            ensure_equal!(
                y.get_type()?,
                z.get_type()?,
                mkerr(IfBranchMismatch(y, z))
            );

            Ok(RetType(y.get_type_move()?))
        }
        EmptyListLit(t) => {
            let t = t.normalize_to_type()?;
            ensure_simple_type!(
                t,
                mkerr(InvalidListType(t.into_normalized()?)),
            );
            let t = t.embed()?;
            Ok(RetExpr(dhall::expr!(List t)))
        }
        NEListLit(xs) => {
            let mut iter = xs.into_iter().enumerate();
            let (_, x) = iter.next().unwrap();
            ensure_simple_type!(
                x.get_type()?,
                mkerr(InvalidListType(x.get_type_move()?.into_normalized()?)),
            );
            for (i, y) in iter {
                ensure_equal!(
                    x.get_type()?,
                    y.get_type()?,
                    mkerr(InvalidListElement(
                        i,
                        x.get_type_move()?.into_normalized()?,
                        y
                    ))
                );
            }
            let t = x.get_type_move()?;
            let t = t.embed()?;
            Ok(RetExpr(dhall::expr!(List t)))
        }
        OldOptionalLit(None, t) => {
            let t = t.normalize()?.embed();
            let e = dhall::subexpr!(None t);
            Ok(RetType(
                type_with(ctx, e)?.into_typed()?.get_type()?.into_owned(),
            ))
        }
        OldOptionalLit(Some(x), t) => {
            let t = t.normalize()?.embed();
            let x = x.normalize()?.embed();
            let e = dhall::subexpr!(Some x : Optional t);
            Ok(RetType(
                type_with(ctx, e)?.into_typed()?.get_type()?.into_owned(),
            ))
        }
        SomeLit(x) => {
            let tx = x.get_type_move()?;
            ensure_simple_type!(
                tx,
                mkerr(InvalidOptionalType(tx.into_normalized()?)),
            );
            let t = tx.embed()?;
            Ok(RetExpr(dhall::expr!(Optional t)))
        }
        RecordType(kts) => {
            let kts: BTreeMap<_, _> = kts
                .into_iter()
                .map(|(x, t)| Ok((x, t.normalize_to_type()?)))
                .collect::<Result<_, _>>()?;
            Ok(RetTypedOrType(
                TypeIntermediate::RecordType(ctx.clone(), kts).typecheck()?,
            ))
        }
        UnionType(kts) => {
            // Check that all types are the same const
            let mut k = None;
            for (x, t) in kts {
                if let Some(t) = t {
                    let k2 = ensure_is_const!(
                        t.get_type()?,
                        mkerr(InvalidFieldType(x, t))
                    );
                    match k {
                        None => k = Some(k2),
                        Some(k1) if k1 != k2 => {
                            return Err(mkerr(InvalidFieldType(x, t)))
                        }
                        Some(_) => {}
                    }
                }
            }
            // An empty union type has type Type;
            // an union type with only unary variants also has type Type
            let k = k.unwrap_or(dhall_core::Const::Type);
            Ok(RetType(const_to_type(k)))
        }
        RecordLit(kvs) => {
            let kts = kvs
                .into_iter()
                .map(|(x, v)| Ok((x, v.get_type_move()?)))
                .collect::<Result<_, _>>()?;
            Ok(RetType(
                TypeIntermediate::RecordType(ctx.clone(), kts)
                    .typecheck()?
                    .normalize_to_type()?,
            ))
        }
        UnionLit(x, v, kvs) => {
            let mut kts: std::collections::BTreeMap<_, _> = kvs
                .into_iter()
                .map(|(x, v)| {
                    let t = match v {
                        Some(x) => Some(x.normalize()?.embed()),
                        None => None,
                    };
                    Ok((x, t))
                })
                .collect::<Result<_, _>>()?;
            let t = v.get_type_move()?.embed()?;
            kts.insert(x, Some(t));
            Ok(RetExpr(UnionType(kts)))
        }
        // Field(r, x) => match &r.get_type()?.0 {
        //     TypeInternal::RecordType(_, _, kts) => match kts.get(&x) {
        //         Some(t) => Ok(RetType(t.clone())),
        //         None => Err(mkerr(MissingRecordField(x, r))),
        //     },
        Field(r, x) => match r.get_type()?.unroll_ref()?.as_ref() {
            RecordType(kts) => match kts.get(&x) {
                Some(t) => Ok(RetExpr(t.unroll().embed_absurd())),
                None => Err(mkerr(MissingRecordField(x, r))),
            },
            _ => {
                let r = r.normalize_to_type()?;
                match r.as_normalized()?.as_expr().as_ref() {
                    UnionType(kts) => match kts.get(&x) {
                        // Constructor has type T -> < x: T, ... >
                        // TODO: use "_" instead of x (i.e. compare types using equivalence)
                        Some(Some(t)) => Ok(RetType(
                            TypeIntermediate::Pi(
                                ctx.clone(),
                                x.clone(),
                                mktype(ctx, t.embed_absurd())?,
                                r,
                            )
                            .typecheck()?
                            .normalize_to_type()?,
                        )),
                        Some(None) => Ok(RetType(r)),
                        None => Err(mkerr(MissingUnionField(
                            x,
                            r.into_normalized()?,
                        ))),
                    },
                    _ => Err(mkerr(NotARecord(x, r.into_normalized()?))),
                }
            }
        },
        Const(c) => Ok(RetType(type_of_const(c))),
        Builtin(b) => Ok(RetExpr(type_of_builtin(b))),
        BoolLit(_) => Ok(RetType(simple_type_from_builtin(Bool))),
        NaturalLit(_) => Ok(RetType(simple_type_from_builtin(Natural))),
        IntegerLit(_) => Ok(RetType(simple_type_from_builtin(Integer))),
        DoubleLit(_) => Ok(RetType(simple_type_from_builtin(Double))),
        // TODO: check type of interpolations
        TextLit(_) => Ok(RetType(simple_type_from_builtin(Text))),
        BinOp(o @ ListAppend, l, r) => {
            match l.get_type()?.unroll_ref()?.as_ref() {
                App(f, _) => match f.as_ref() {
                    Builtin(List) => {}
                    _ => return Err(mkerr(BinOpTypeMismatch(o, l))),
                },
                _ => return Err(mkerr(BinOpTypeMismatch(o, l))),
            }

            ensure_equal!(
                l.get_type()?,
                r.get_type()?,
                mkerr(BinOpTypeMismatch(o, r))
            );

            Ok(RetType(l.get_type()?.into_owned()))
        }
        BinOp(o, l, r) => {
            let t = simple_type_from_builtin(match o {
                BoolAnd => Bool,
                BoolOr => Bool,
                BoolEQ => Bool,
                BoolNE => Bool,
                NaturalPlus => Natural,
                NaturalTimes => Natural,
                TextAppend => Text,
                ListAppend => unreachable!(),
                _ => return Err(mkerr(Unimplemented)),
            });

            ensure_equal!(l.get_type()?, &t, mkerr(BinOpTypeMismatch(o, l)));

            ensure_equal!(r.get_type()?, &t, mkerr(BinOpTypeMismatch(o, r)));

            Ok(RetType(t))
        }
        _ => Err(mkerr(Unimplemented)),
    }
}

/// `typeOf` is the same as `type_with` with an empty context, meaning that the
/// expression must be closed (i.e. no free variables), otherwise type-checking
/// will fail.
fn type_of(
    e: SubExpr<X, Normalized<'static>>,
) -> Result<Typed<'static>, TypeError> {
    let ctx = TypecheckContext::new();
    let e = type_with(&ctx, e)?.into_typed()?;
    // Ensure the inferred type isn't SuperType
    e.get_type()?.as_normalized()?;
    Ok(e)
}

/// The specific type error
#[derive(Debug)]
pub(crate) enum TypeMessage<'a> {
    UnboundVariable(V<Label>),
    InvalidInputType(Normalized<'a>),
    InvalidOutputType(Normalized<'a>),
    NotAFunction(TypedOrType),
    TypeMismatch(TypedOrType, Normalized<'a>, TypedOrType),
    AnnotMismatch(TypedOrType, Normalized<'a>),
    Untyped,
    InvalidListElement(usize, Normalized<'a>, TypedOrType),
    InvalidListType(Normalized<'a>),
    InvalidOptionalType(Normalized<'a>),
    InvalidPredicate(TypedOrType),
    IfBranchMismatch(TypedOrType, TypedOrType),
    IfBranchMustBeTerm(bool, TypedOrType),
    InvalidFieldType(Label, TypedOrType),
    NotARecord(Label, Normalized<'a>),
    MissingRecordField(Label, TypedOrType),
    MissingUnionField(Label, Normalized<'a>),
    BinOpTypeMismatch(BinOp, TypedOrType),
    NoDependentTypes(Normalized<'a>, Normalized<'a>),
    Unimplemented,
}

/// A structured type error that includes context
#[derive(Debug)]
pub struct TypeError {
    context: TypecheckContext,
    current: SubExpr<X, Normalized<'static>>,
    type_message: TypeMessage<'static>,
}

impl TypeError {
    pub(crate) fn new(
        context: &TypecheckContext,
        current: SubExpr<X, Normalized<'static>>,
        type_message: TypeMessage<'static>,
    ) -> Self {
        TypeError {
            context: context.clone(),
            current,
            type_message,
        }
    }
}

impl From<TypeError> for std::option::NoneError {
    fn from(_: TypeError) -> std::option::NoneError {
        std::option::NoneError
    }
}

impl ::std::error::Error for TypeMessage<'static> {
    fn description(&self) -> &str {
        match *self {
            // UnboundVariable => "Unbound variable",
            InvalidInputType(_) => "Invalid function input",
            InvalidOutputType(_) => "Invalid function output",
            NotAFunction(_) => "Not a function",
            TypeMismatch(_, _, _) => "Wrong type of function argument",
            _ => "Unhandled error",
        }
    }
}

impl fmt::Display for TypeMessage<'static> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self {
            // UnboundVariable(_) => {
            //     f.write_str(include_str!("errors/UnboundVariable.txt"))
            // }
            // TypeMismatch(e0, e1, e2) => {
            //     let template = include_str!("errors/TypeMismatch.txt");
            //     let s = template
            //         .replace("$txt0", &format!("{}", e0.as_expr()))
            //         .replace("$txt1", &format!("{}", e1.as_expr()))
            //         .replace("$txt2", &format!("{}", e2.as_expr()))
            //         .replace(
            //             "$txt3",
            //             &format!(
            //                 "{}",
            //                 e2.get_type()
            //                     .unwrap()
            //                     .as_normalized()
            //                     .unwrap()
            //                     .as_expr()
            //             ),
            //         );
            //     f.write_str(&s)
            // }
            _ => f.write_str("Unhandled error message"),
        }
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use crate::traits::DynamicType;
    use dhall_core::*;
    use dhall_generator as dhall;

    fn typecheck_using_external_dhall(
        expr: &SubExpr<X, X>,
    ) -> Result<SubExpr<X, X>, crate::error::Error> {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let expr_str = expr.to_string();
        let mut child = Command::new("dhall")
            .arg("type")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        {
            let stdin = child.stdin.as_mut().unwrap();
            stdin.write_all(expr_str.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            // I'm lazy
            Err(crate::error::Error::Deserialize(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))?
        }
        let output_str = String::from_utf8_lossy(&output.stdout);
        let ty = crate::expr::Parsed::parse_str(&output_str)?
            .skip_resolve()?
            .skip_typecheck()
            .skip_normalize()
            .into_expr();
        Ok(ty)
    }

    prop_compose! {
        fn label_strategy()(id in 0usize..2usize) -> Label {
            let name = match id {
                0 => "a",
                _ => "b",
            };
            name.into()
        }
    }

    fn type_strategy() -> impl Strategy<Value = SubExpr<X, X>> {
        let leaf = prop_oneof![
            Just(rc(ExprF::Builtin(Builtin::Bool))),
            Just(rc(ExprF::Const(Const::Type))),
            Just(rc(ExprF::Const(Const::Kind))),
            (label_strategy(), 0usize..3usize)
                .prop_map(|(name, id)| { rc(ExprF::Var(V(name, id))) }),
        ];
        leaf.prop_recursive(
            5,  // 8 levels deep
            32, // Shoot for maximum size of 256 nodes
            2,  // We put up to 4 items per collection
            |inner| {
                prop_oneof![
                    (label_strategy(), inner.clone(), inner.clone())
                        .prop_map(|(x, t, e)| rc(ExprF::Pi(x, t, e))),
                    inner.clone().prop_map(|e| dhall::subexpr!({ x: e })),
                ]
            },
        )
    }

    fn expr_strategy() -> impl Strategy<Value = SubExpr<X, X>> {
        let leaf = prop_oneof![
            (label_strategy(), 0usize..3usize)
                .prop_map(|(name, id)| { rc(ExprF::Var(V(name, id))) }),
            any::<bool>().prop_map(|b| rc(ExprF::BoolLit(b))),
        ];
        leaf.prop_recursive(
            8,  // 8 levels deep
            256, // Shoot for maximum size of 256 nodes
            2,  // We put up to 4 items per collection
            |inner| {
                prop_oneof![
                    (label_strategy(), type_strategy(), inner.clone())
                        .prop_map(|(x, t, e)| rc(ExprF::Lam(x, t, e))),
                    (inner.clone(), inner.clone())
                        .prop_map(|(f, a)| rc(ExprF::App(f, a))),
                    inner.clone().prop_map(|e| rc(ExprF::Field(e, "x".into()))),
                    inner.clone().prop_map(|e| dhall::subexpr!({ x = e })),
                ]
            },
        )
    }

    // proptest! {
    //     #![proptest_config(ProptestConfig {
    //         max_global_rejects: 1000000,
    //         cases: 256,
    //         ..ProptestConfig::default()
    //     })]
    //     #[test]
    //     fn proptest_compare(expr in expr_strategy()) {
    //         let output_expr_err = typecheck_using_external_dhall(&expr);
    //         prop_assume!(output_expr_err.is_ok());
    //         let output_expr = output_expr_err.unwrap();
    //         let expected: SubExpr<X, X> = super::type_of(expr.embed_absurd())
    //             .unwrap()
    //             .get_type()
    //             .unwrap()
    //             .into_owned()
    //             .into_normalized()
    //             .unwrap()
    //             .into_expr();
    //         prop_assert_eq!(output_expr, expected);
    //     }
    // }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            ..ProptestConfig::default()
        })]
        #[test]
        fn proptest_alone(expr in expr_strategy()) {
            super::type_of(expr.embed_absurd());
        }
    }
}

#[cfg(test)]
mod spec_tests {
    #![rustfmt::skip]

    macro_rules! tc_success {
        ($name:ident, $path:expr) => {
            make_spec_test!(Typecheck, Success, $name, $path);
        };
    }
    macro_rules! tc_failure {
        ($name:ident, $path:expr) => {
            make_spec_test!(Typecheck, Failure, $name, $path);
        };
    }

    macro_rules! ti_success {
        ($name:ident, $path:expr) => {
            make_spec_test!(TypeInference, Success, $name, $path);
        };
    }
    // macro_rules! ti_failure {
    //     ($name:ident, $path:expr) => {
    //         make_spec_test!(TypeInference, Failure, $name, $path);
    //     };
    // }

    // tc_success!(tc_success_accessEncodedType, "accessEncodedType");
    // tc_success!(tc_success_accessType, "accessType");
    tc_success!(tc_success_prelude_Bool_and_0, "prelude/Bool/and/0");
    tc_success!(tc_success_prelude_Bool_and_1, "prelude/Bool/and/1");
    tc_success!(tc_success_prelude_Bool_build_0, "prelude/Bool/build/0");
    tc_success!(tc_success_prelude_Bool_build_1, "prelude/Bool/build/1");
    tc_success!(tc_success_prelude_Bool_even_0, "prelude/Bool/even/0");
    tc_success!(tc_success_prelude_Bool_even_1, "prelude/Bool/even/1");
    tc_success!(tc_success_prelude_Bool_even_2, "prelude/Bool/even/2");
    tc_success!(tc_success_prelude_Bool_even_3, "prelude/Bool/even/3");
    tc_success!(tc_success_prelude_Bool_fold_0, "prelude/Bool/fold/0");
    tc_success!(tc_success_prelude_Bool_fold_1, "prelude/Bool/fold/1");
    tc_success!(tc_success_prelude_Bool_not_0, "prelude/Bool/not/0");
    tc_success!(tc_success_prelude_Bool_not_1, "prelude/Bool/not/1");
    tc_success!(tc_success_prelude_Bool_odd_0, "prelude/Bool/odd/0");
    tc_success!(tc_success_prelude_Bool_odd_1, "prelude/Bool/odd/1");
    tc_success!(tc_success_prelude_Bool_odd_2, "prelude/Bool/odd/2");
    tc_success!(tc_success_prelude_Bool_odd_3, "prelude/Bool/odd/3");
    tc_success!(tc_success_prelude_Bool_or_0, "prelude/Bool/or/0");
    tc_success!(tc_success_prelude_Bool_or_1, "prelude/Bool/or/1");
    tc_success!(tc_success_prelude_Bool_show_0, "prelude/Bool/show/0");
    tc_success!(tc_success_prelude_Bool_show_1, "prelude/Bool/show/1");
    // tc_success!(tc_success_prelude_Double_show_0, "prelude/Double/show/0");
    // tc_success!(tc_success_prelude_Double_show_1, "prelude/Double/show/1");
    // tc_success!(tc_success_prelude_Integer_show_0, "prelude/Integer/show/0");
    // tc_success!(tc_success_prelude_Integer_show_1, "prelude/Integer/show/1");
    // tc_success!(tc_success_prelude_Integer_toDouble_0, "prelude/Integer/toDouble/0");
    // tc_success!(tc_success_prelude_Integer_toDouble_1, "prelude/Integer/toDouble/1");
    tc_success!(tc_success_prelude_List_all_0, "prelude/List/all/0");
    tc_success!(tc_success_prelude_List_all_1, "prelude/List/all/1");
    tc_success!(tc_success_prelude_List_any_0, "prelude/List/any/0");
    tc_success!(tc_success_prelude_List_any_1, "prelude/List/any/1");
    tc_success!(tc_success_prelude_List_build_0, "prelude/List/build/0");
    tc_success!(tc_success_prelude_List_build_1, "prelude/List/build/1");
    tc_success!(tc_success_prelude_List_concat_0, "prelude/List/concat/0");
    tc_success!(tc_success_prelude_List_concat_1, "prelude/List/concat/1");
    tc_success!(tc_success_prelude_List_concatMap_0, "prelude/List/concatMap/0");
    tc_success!(tc_success_prelude_List_concatMap_1, "prelude/List/concatMap/1");
    tc_success!(tc_success_prelude_List_filter_0, "prelude/List/filter/0");
    tc_success!(tc_success_prelude_List_filter_1, "prelude/List/filter/1");
    tc_success!(tc_success_prelude_List_fold_0, "prelude/List/fold/0");
    tc_success!(tc_success_prelude_List_fold_1, "prelude/List/fold/1");
    tc_success!(tc_success_prelude_List_fold_2, "prelude/List/fold/2");
    tc_success!(tc_success_prelude_List_generate_0, "prelude/List/generate/0");
    tc_success!(tc_success_prelude_List_generate_1, "prelude/List/generate/1");
    tc_success!(tc_success_prelude_List_head_0, "prelude/List/head/0");
    tc_success!(tc_success_prelude_List_head_1, "prelude/List/head/1");
    tc_success!(tc_success_prelude_List_indexed_0, "prelude/List/indexed/0");
    tc_success!(tc_success_prelude_List_indexed_1, "prelude/List/indexed/1");
    tc_success!(tc_success_prelude_List_iterate_0, "prelude/List/iterate/0");
    tc_success!(tc_success_prelude_List_iterate_1, "prelude/List/iterate/1");
    tc_success!(tc_success_prelude_List_last_0, "prelude/List/last/0");
    tc_success!(tc_success_prelude_List_last_1, "prelude/List/last/1");
    tc_success!(tc_success_prelude_List_length_0, "prelude/List/length/0");
    tc_success!(tc_success_prelude_List_length_1, "prelude/List/length/1");
    tc_success!(tc_success_prelude_List_map_0, "prelude/List/map/0");
    tc_success!(tc_success_prelude_List_map_1, "prelude/List/map/1");
    tc_success!(tc_success_prelude_List_null_0, "prelude/List/null/0");
    tc_success!(tc_success_prelude_List_null_1, "prelude/List/null/1");
    tc_success!(tc_success_prelude_List_replicate_0, "prelude/List/replicate/0");
    tc_success!(tc_success_prelude_List_replicate_1, "prelude/List/replicate/1");
    tc_success!(tc_success_prelude_List_reverse_0, "prelude/List/reverse/0");
    tc_success!(tc_success_prelude_List_reverse_1, "prelude/List/reverse/1");
    tc_success!(tc_success_prelude_List_shifted_0, "prelude/List/shifted/0");
    tc_success!(tc_success_prelude_List_shifted_1, "prelude/List/shifted/1");
    tc_success!(tc_success_prelude_List_unzip_0, "prelude/List/unzip/0");
    tc_success!(tc_success_prelude_List_unzip_1, "prelude/List/unzip/1");
    tc_success!(tc_success_prelude_Monoid_00, "prelude/Monoid/00");
    tc_success!(tc_success_prelude_Monoid_01, "prelude/Monoid/01");
    tc_success!(tc_success_prelude_Monoid_02, "prelude/Monoid/02");
    tc_success!(tc_success_prelude_Monoid_03, "prelude/Monoid/03");
    tc_success!(tc_success_prelude_Monoid_04, "prelude/Monoid/04");
    tc_success!(tc_success_prelude_Monoid_05, "prelude/Monoid/05");
    tc_success!(tc_success_prelude_Monoid_06, "prelude/Monoid/06");
    tc_success!(tc_success_prelude_Monoid_07, "prelude/Monoid/07");
    tc_success!(tc_success_prelude_Monoid_08, "prelude/Monoid/08");
    tc_success!(tc_success_prelude_Monoid_09, "prelude/Monoid/09");
    tc_success!(tc_success_prelude_Monoid_10, "prelude/Monoid/10");
    tc_success!(tc_success_prelude_Natural_build_0, "prelude/Natural/build/0");
    tc_success!(tc_success_prelude_Natural_build_1, "prelude/Natural/build/1");
    tc_success!(tc_success_prelude_Natural_enumerate_0, "prelude/Natural/enumerate/0");
    tc_success!(tc_success_prelude_Natural_enumerate_1, "prelude/Natural/enumerate/1");
    tc_success!(tc_success_prelude_Natural_even_0, "prelude/Natural/even/0");
    tc_success!(tc_success_prelude_Natural_even_1, "prelude/Natural/even/1");
    tc_success!(tc_success_prelude_Natural_fold_0, "prelude/Natural/fold/0");
    tc_success!(tc_success_prelude_Natural_fold_1, "prelude/Natural/fold/1");
    tc_success!(tc_success_prelude_Natural_fold_2, "prelude/Natural/fold/2");
    tc_success!(tc_success_prelude_Natural_isZero_0, "prelude/Natural/isZero/0");
    tc_success!(tc_success_prelude_Natural_isZero_1, "prelude/Natural/isZero/1");
    tc_success!(tc_success_prelude_Natural_odd_0, "prelude/Natural/odd/0");
    tc_success!(tc_success_prelude_Natural_odd_1, "prelude/Natural/odd/1");
    tc_success!(tc_success_prelude_Natural_product_0, "prelude/Natural/product/0");
    tc_success!(tc_success_prelude_Natural_product_1, "prelude/Natural/product/1");
    // tc_success!(tc_success_prelude_Natural_show_0, "prelude/Natural/show/0");
    // tc_success!(tc_success_prelude_Natural_show_1, "prelude/Natural/show/1");
    tc_success!(tc_success_prelude_Natural_sum_0, "prelude/Natural/sum/0");
    tc_success!(tc_success_prelude_Natural_sum_1, "prelude/Natural/sum/1");
    // tc_success!(tc_success_prelude_Natural_toDouble_0, "prelude/Natural/toDouble/0");
    // tc_success!(tc_success_prelude_Natural_toDouble_1, "prelude/Natural/toDouble/1");
    // tc_success!(tc_success_prelude_Natural_toInteger_0, "prelude/Natural/toInteger/0");
    // tc_success!(tc_success_prelude_Natural_toInteger_1, "prelude/Natural/toInteger/1");
    tc_success!(tc_success_prelude_Optional_all_0, "prelude/Optional/all/0");
    tc_success!(tc_success_prelude_Optional_all_1, "prelude/Optional/all/1");
    tc_success!(tc_success_prelude_Optional_any_0, "prelude/Optional/any/0");
    tc_success!(tc_success_prelude_Optional_any_1, "prelude/Optional/any/1");
    // tc_success!(tc_success_prelude_Optional_build_0, "prelude/Optional/build/0");
    // tc_success!(tc_success_prelude_Optional_build_1, "prelude/Optional/build/1");
    tc_success!(tc_success_prelude_Optional_concat_0, "prelude/Optional/concat/0");
    tc_success!(tc_success_prelude_Optional_concat_1, "prelude/Optional/concat/1");
    tc_success!(tc_success_prelude_Optional_concat_2, "prelude/Optional/concat/2");
    // tc_success!(tc_success_prelude_Optional_filter_0, "prelude/Optional/filter/0");
    // tc_success!(tc_success_prelude_Optional_filter_1, "prelude/Optional/filter/1");
    tc_success!(tc_success_prelude_Optional_fold_0, "prelude/Optional/fold/0");
    tc_success!(tc_success_prelude_Optional_fold_1, "prelude/Optional/fold/1");
    tc_success!(tc_success_prelude_Optional_head_0, "prelude/Optional/head/0");
    tc_success!(tc_success_prelude_Optional_head_1, "prelude/Optional/head/1");
    tc_success!(tc_success_prelude_Optional_head_2, "prelude/Optional/head/2");
    tc_success!(tc_success_prelude_Optional_last_0, "prelude/Optional/last/0");
    tc_success!(tc_success_prelude_Optional_last_1, "prelude/Optional/last/1");
    tc_success!(tc_success_prelude_Optional_last_2, "prelude/Optional/last/2");
    tc_success!(tc_success_prelude_Optional_length_0, "prelude/Optional/length/0");
    tc_success!(tc_success_prelude_Optional_length_1, "prelude/Optional/length/1");
    tc_success!(tc_success_prelude_Optional_map_0, "prelude/Optional/map/0");
    tc_success!(tc_success_prelude_Optional_map_1, "prelude/Optional/map/1");
    tc_success!(tc_success_prelude_Optional_null_0, "prelude/Optional/null/0");
    tc_success!(tc_success_prelude_Optional_null_1, "prelude/Optional/null/1");
    tc_success!(tc_success_prelude_Optional_toList_0, "prelude/Optional/toList/0");
    tc_success!(tc_success_prelude_Optional_toList_1, "prelude/Optional/toList/1");
    tc_success!(tc_success_prelude_Optional_unzip_0, "prelude/Optional/unzip/0");
    tc_success!(tc_success_prelude_Optional_unzip_1, "prelude/Optional/unzip/1");
    tc_success!(tc_success_prelude_Text_concat_0, "prelude/Text/concat/0");
    tc_success!(tc_success_prelude_Text_concat_1, "prelude/Text/concat/1");
    // tc_success!(tc_success_prelude_Text_concatMap_0, "prelude/Text/concatMap/0");
    // tc_success!(tc_success_prelude_Text_concatMap_1, "prelude/Text/concatMap/1");
    // tc_success!(tc_success_prelude_Text_concatMapSep_0, "prelude/Text/concatMapSep/0");
    // tc_success!(tc_success_prelude_Text_concatMapSep_1, "prelude/Text/concatMapSep/1");
    // tc_success!(tc_success_prelude_Text_concatSep_0, "prelude/Text/concatSep/0");
    // tc_success!(tc_success_prelude_Text_concatSep_1, "prelude/Text/concatSep/1");
    tc_success!(tc_success_recordOfRecordOfTypes, "recordOfRecordOfTypes");
    tc_success!(tc_success_recordOfTypes, "recordOfTypes");
    // tc_success!(tc_success_simple_access_0, "simple/access/0");
    // tc_success!(tc_success_simple_access_1, "simple/access/1");
    // tc_success!(tc_success_simple_alternativesAreTypes, "simple/alternativesAreTypes");
    // tc_success!(tc_success_simple_anonymousFunctionsInTypes, "simple/anonymousFunctionsInTypes");
    // tc_success!(tc_success_simple_fieldsAreTypes, "simple/fieldsAreTypes");
    // tc_success!(tc_success_simple_kindParameter, "simple/kindParameter");
    // tc_success!(tc_success_simple_mergeEquivalence, "simple/mergeEquivalence");
    // tc_success!(tc_success_simple_mixedFieldAccess, "simple/mixedFieldAccess");
    tc_success!(tc_success_simple_unionsOfTypes, "simple/unionsOfTypes");

    tc_failure!(tc_failure_combineMixedRecords, "combineMixedRecords");
    // tc_failure!(tc_failure_duplicateFields, "duplicateFields");
    tc_failure!(tc_failure_hurkensParadox, "hurkensParadox");
    tc_failure!(tc_failure_mixedUnions, "mixedUnions");
    tc_failure!(tc_failure_preferMixedRecords, "preferMixedRecords");
    tc_failure!(tc_failure_unit_FunctionApplicationArgumentNotMatch, "unit/FunctionApplicationArgumentNotMatch");
    tc_failure!(tc_failure_unit_FunctionApplicationIsNotFunction, "unit/FunctionApplicationIsNotFunction");
    tc_failure!(tc_failure_unit_FunctionArgumentTypeNotAType, "unit/FunctionArgumentTypeNotAType");
    tc_failure!(tc_failure_unit_FunctionDependentType, "unit/FunctionDependentType");
    tc_failure!(tc_failure_unit_FunctionDependentType2, "unit/FunctionDependentType2");
    tc_failure!(tc_failure_unit_FunctionTypeArgumentTypeNotAType, "unit/FunctionTypeArgumentTypeNotAType");
    tc_failure!(tc_failure_unit_FunctionTypeKindSort, "unit/FunctionTypeKindSort");
    tc_failure!(tc_failure_unit_FunctionTypeTypeKind, "unit/FunctionTypeTypeKind");
    tc_failure!(tc_failure_unit_FunctionTypeTypeSort, "unit/FunctionTypeTypeSort");
    tc_failure!(tc_failure_unit_IfBranchesNotMatch, "unit/IfBranchesNotMatch");
    tc_failure!(tc_failure_unit_IfBranchesNotType, "unit/IfBranchesNotType");
    tc_failure!(tc_failure_unit_IfNotBool, "unit/IfNotBool");
    tc_failure!(tc_failure_unit_LetWithWrongAnnotation, "unit/LetWithWrongAnnotation");
    tc_failure!(tc_failure_unit_ListLiteralEmptyNotType, "unit/ListLiteralEmptyNotType");
    tc_failure!(tc_failure_unit_ListLiteralNotType, "unit/ListLiteralNotType");
    tc_failure!(tc_failure_unit_ListLiteralTypesNotMatch, "unit/ListLiteralTypesNotMatch");
    tc_failure!(tc_failure_unit_MergeAlternativeHasNoHandler, "unit/MergeAlternativeHasNoHandler");
    tc_failure!(tc_failure_unit_MergeAnnotationNotType, "unit/MergeAnnotationNotType");
    tc_failure!(tc_failure_unit_MergeEmptyWithoutAnnotation, "unit/MergeEmptyWithoutAnnotation");
    tc_failure!(tc_failure_unit_MergeHandlerNotFunction, "unit/MergeHandlerNotFunction");
    tc_failure!(tc_failure_unit_MergeHandlerNotInUnion, "unit/MergeHandlerNotInUnion");
    tc_failure!(tc_failure_unit_MergeHandlerNotMatchAlternativeType, "unit/MergeHandlerNotMatchAlternativeType");
    tc_failure!(tc_failure_unit_MergeHandlersWithDifferentType, "unit/MergeHandlersWithDifferentType");
    tc_failure!(tc_failure_unit_MergeLhsNotRecord, "unit/MergeLhsNotRecord");
    tc_failure!(tc_failure_unit_MergeRhsNotUnion, "unit/MergeRhsNotUnion");
    tc_failure!(tc_failure_unit_MergeWithWrongAnnotation, "unit/MergeWithWrongAnnotation");
    tc_failure!(tc_failure_unit_OperatorAndNotBool, "unit/OperatorAndNotBool");
    tc_failure!(tc_failure_unit_OperatorEqualNotBool, "unit/OperatorEqualNotBool");
    tc_failure!(tc_failure_unit_OperatorListConcatenateLhsNotList, "unit/OperatorListConcatenateLhsNotList");
    tc_failure!(tc_failure_unit_OperatorListConcatenateListsNotMatch, "unit/OperatorListConcatenateListsNotMatch");
    tc_failure!(tc_failure_unit_OperatorListConcatenateRhsNotList, "unit/OperatorListConcatenateRhsNotList");
    tc_failure!(tc_failure_unit_OperatorNotEqualNotBool, "unit/OperatorNotEqualNotBool");
    tc_failure!(tc_failure_unit_OperatorOrNotBool, "unit/OperatorOrNotBool");
    tc_failure!(tc_failure_unit_OperatorPlusNotNatural, "unit/OperatorPlusNotNatural");
    tc_failure!(tc_failure_unit_OperatorTextConcatenateLhsNotText, "unit/OperatorTextConcatenateLhsNotText");
    tc_failure!(tc_failure_unit_OperatorTextConcatenateRhsNotText, "unit/OperatorTextConcatenateRhsNotText");
    tc_failure!(tc_failure_unit_OperatorTimesNotNatural, "unit/OperatorTimesNotNatural");
    tc_failure!(tc_failure_unit_RecordMixedKinds, "unit/RecordMixedKinds");
    tc_failure!(tc_failure_unit_RecordMixedKinds2, "unit/RecordMixedKinds2");
    tc_failure!(tc_failure_unit_RecordMixedKinds3, "unit/RecordMixedKinds3");
    tc_failure!(tc_failure_unit_RecordProjectionEmpty, "unit/RecordProjectionEmpty");
    tc_failure!(tc_failure_unit_RecordProjectionNotPresent, "unit/RecordProjectionNotPresent");
    tc_failure!(tc_failure_unit_RecordProjectionNotRecord, "unit/RecordProjectionNotRecord");
    tc_failure!(tc_failure_unit_RecordSelectionEmpty, "unit/RecordSelectionEmpty");
    tc_failure!(tc_failure_unit_RecordSelectionNotPresent, "unit/RecordSelectionNotPresent");
    tc_failure!(tc_failure_unit_RecordSelectionNotRecord, "unit/RecordSelectionNotRecord");
    tc_failure!(tc_failure_unit_RecordSelectionTypeNotUnionType, "unit/RecordSelectionTypeNotUnionType");
    tc_failure!(tc_failure_unit_RecordTypeMixedKinds, "unit/RecordTypeMixedKinds");
    tc_failure!(tc_failure_unit_RecordTypeMixedKinds2, "unit/RecordTypeMixedKinds2");
    tc_failure!(tc_failure_unit_RecordTypeMixedKinds3, "unit/RecordTypeMixedKinds3");
    tc_failure!(tc_failure_unit_RecordTypeValueMember, "unit/RecordTypeValueMember");
    tc_failure!(tc_failure_unit_RecursiveRecordMergeLhsNotRecord, "unit/RecursiveRecordMergeLhsNotRecord");
    tc_failure!(tc_failure_unit_RecursiveRecordMergeMixedKinds, "unit/RecursiveRecordMergeMixedKinds");
    tc_failure!(tc_failure_unit_RecursiveRecordMergeOverlapping, "unit/RecursiveRecordMergeOverlapping");
    tc_failure!(tc_failure_unit_RecursiveRecordMergeRhsNotRecord, "unit/RecursiveRecordMergeRhsNotRecord");
    tc_failure!(tc_failure_unit_RecursiveRecordTypeMergeLhsNotRecordType, "unit/RecursiveRecordTypeMergeLhsNotRecordType");
    tc_failure!(tc_failure_unit_RecursiveRecordTypeMergeOverlapping, "unit/RecursiveRecordTypeMergeOverlapping");
    tc_failure!(tc_failure_unit_RecursiveRecordTypeMergeRhsNotRecordType, "unit/RecursiveRecordTypeMergeRhsNotRecordType");
    tc_failure!(tc_failure_unit_RightBiasedRecordMergeLhsNotRecord, "unit/RightBiasedRecordMergeLhsNotRecord");
    tc_failure!(tc_failure_unit_RightBiasedRecordMergeMixedKinds, "unit/RightBiasedRecordMergeMixedKinds");
    tc_failure!(tc_failure_unit_RightBiasedRecordMergeMixedKinds2, "unit/RightBiasedRecordMergeMixedKinds2");
    tc_failure!(tc_failure_unit_RightBiasedRecordMergeMixedKinds3, "unit/RightBiasedRecordMergeMixedKinds3");
    tc_failure!(tc_failure_unit_RightBiasedRecordMergeRhsNotRecord, "unit/RightBiasedRecordMergeRhsNotRecord");
    tc_failure!(tc_failure_unit_SomeNotType, "unit/SomeNotType");
    tc_failure!(tc_failure_unit_Sort, "unit/Sort");
    // tc_failure!(tc_failure_unit_TextLiteralInterpolateNotText, "unit/TextLiteralInterpolateNotText");
    tc_failure!(tc_failure_unit_TypeAnnotationWrong, "unit/TypeAnnotationWrong");
    tc_failure!(tc_failure_unit_UnionConstructorFieldNotPresent, "unit/UnionConstructorFieldNotPresent");
    tc_failure!(tc_failure_unit_UnionTypeMixedKinds, "unit/UnionTypeMixedKinds");
    tc_failure!(tc_failure_unit_UnionTypeMixedKinds2, "unit/UnionTypeMixedKinds2");
    tc_failure!(tc_failure_unit_UnionTypeMixedKinds3, "unit/UnionTypeMixedKinds3");
    tc_failure!(tc_failure_unit_UnionTypeNotType, "unit/UnionTypeNotType");
    tc_failure!(tc_failure_unit_VariableFree, "unit/VariableFree");

    // ti_success!(ti_success_simple_alternativesAreTypes, "simple/alternativesAreTypes");
    // ti_success!(ti_success_simple_kindParameter, "simple/kindParameter");
    ti_success!(ti_success_unit_Bool, "unit/Bool");
    ti_success!(ti_success_unit_Double, "unit/Double");
    ti_success!(ti_success_unit_DoubleLiteral, "unit/DoubleLiteral");
    // ti_success!(ti_success_unit_DoubleShow, "unit/DoubleShow");
    ti_success!(ti_success_unit_False, "unit/False");
    ti_success!(ti_success_unit_Function, "unit/Function");
    ti_success!(ti_success_unit_FunctionApplication, "unit/FunctionApplication");
    ti_success!(ti_success_unit_FunctionNamedArg, "unit/FunctionNamedArg");
    ti_success!(ti_success_unit_FunctionTypeKindKind, "unit/FunctionTypeKindKind");
    ti_success!(ti_success_unit_FunctionTypeKindTerm, "unit/FunctionTypeKindTerm");
    ti_success!(ti_success_unit_FunctionTypeKindType, "unit/FunctionTypeKindType");
    ti_success!(ti_success_unit_FunctionTypeTermTerm, "unit/FunctionTypeTermTerm");
    ti_success!(ti_success_unit_FunctionTypeTypeTerm, "unit/FunctionTypeTypeTerm");
    ti_success!(ti_success_unit_FunctionTypeTypeType, "unit/FunctionTypeTypeType");
    ti_success!(ti_success_unit_FunctionTypeUsingArgument, "unit/FunctionTypeUsingArgument");
    ti_success!(ti_success_unit_If, "unit/If");
    ti_success!(ti_success_unit_IfNormalizeArguments, "unit/IfNormalizeArguments");
    ti_success!(ti_success_unit_Integer, "unit/Integer");
    ti_success!(ti_success_unit_IntegerLiteral, "unit/IntegerLiteral");
    // ti_success!(ti_success_unit_IntegerShow, "unit/IntegerShow");
    // ti_success!(ti_success_unit_IntegerToDouble, "unit/IntegerToDouble");
    ti_success!(ti_success_unit_Kind, "unit/Kind");
    ti_success!(ti_success_unit_Let, "unit/Let");
    ti_success!(ti_success_unit_LetNestedTypeSynonym, "unit/LetNestedTypeSynonym");
    ti_success!(ti_success_unit_LetTypeSynonym, "unit/LetTypeSynonym");
    ti_success!(ti_success_unit_LetWithAnnotation, "unit/LetWithAnnotation");
    ti_success!(ti_success_unit_List, "unit/List");
    ti_success!(ti_success_unit_ListBuild, "unit/ListBuild");
    ti_success!(ti_success_unit_ListFold, "unit/ListFold");
    ti_success!(ti_success_unit_ListHead, "unit/ListHead");
    ti_success!(ti_success_unit_ListIndexed, "unit/ListIndexed");
    ti_success!(ti_success_unit_ListLast, "unit/ListLast");
    ti_success!(ti_success_unit_ListLength, "unit/ListLength");
    ti_success!(ti_success_unit_ListLiteralEmpty, "unit/ListLiteralEmpty");
    ti_success!(ti_success_unit_ListLiteralNormalizeArguments, "unit/ListLiteralNormalizeArguments");
    ti_success!(ti_success_unit_ListLiteralOne, "unit/ListLiteralOne");
    ti_success!(ti_success_unit_ListReverse, "unit/ListReverse");
    // ti_success!(ti_success_unit_MergeEmptyUnion, "unit/MergeEmptyUnion");
    // ti_success!(ti_success_unit_MergeOne, "unit/MergeOne");
    // ti_success!(ti_success_unit_MergeOneWithAnnotation, "unit/MergeOneWithAnnotation");
    ti_success!(ti_success_unit_Natural, "unit/Natural");
    ti_success!(ti_success_unit_NaturalBuild, "unit/NaturalBuild");
    ti_success!(ti_success_unit_NaturalEven, "unit/NaturalEven");
    ti_success!(ti_success_unit_NaturalFold, "unit/NaturalFold");
    ti_success!(ti_success_unit_NaturalIsZero, "unit/NaturalIsZero");
    ti_success!(ti_success_unit_NaturalLiteral, "unit/NaturalLiteral");
    ti_success!(ti_success_unit_NaturalOdd, "unit/NaturalOdd");
    // ti_success!(ti_success_unit_NaturalShow, "unit/NaturalShow");
    // ti_success!(ti_success_unit_NaturalToInteger, "unit/NaturalToInteger");
    // ti_success!(ti_success_unit_None, "unit/None");
    ti_success!(ti_success_unit_OldOptionalNone, "unit/OldOptionalNone");
    // ti_success!(ti_success_unit_OldOptionalTrue, "unit/OldOptionalTrue");
    ti_success!(ti_success_unit_OperatorAnd, "unit/OperatorAnd");
    ti_success!(ti_success_unit_OperatorAndNormalizeArguments, "unit/OperatorAndNormalizeArguments");
    ti_success!(ti_success_unit_OperatorEqual, "unit/OperatorEqual");
    ti_success!(ti_success_unit_OperatorEqualNormalizeArguments, "unit/OperatorEqualNormalizeArguments");
    ti_success!(ti_success_unit_OperatorListConcatenate, "unit/OperatorListConcatenate");
    ti_success!(ti_success_unit_OperatorListConcatenateNormalizeArguments, "unit/OperatorListConcatenateNormalizeArguments");
    ti_success!(ti_success_unit_OperatorNotEqual, "unit/OperatorNotEqual");
    ti_success!(ti_success_unit_OperatorNotEqualNormalizeArguments, "unit/OperatorNotEqualNormalizeArguments");
    ti_success!(ti_success_unit_OperatorOr, "unit/OperatorOr");
    ti_success!(ti_success_unit_OperatorOrNormalizeArguments, "unit/OperatorOrNormalizeArguments");
    ti_success!(ti_success_unit_OperatorPlus, "unit/OperatorPlus");
    ti_success!(ti_success_unit_OperatorPlusNormalizeArguments, "unit/OperatorPlusNormalizeArguments");
    ti_success!(ti_success_unit_OperatorTextConcatenate, "unit/OperatorTextConcatenate");
    ti_success!(ti_success_unit_OperatorTextConcatenateNormalizeArguments, "unit/OperatorTextConcatenateNormalizeArguments");
    ti_success!(ti_success_unit_OperatorTimes, "unit/OperatorTimes");
    ti_success!(ti_success_unit_OperatorTimesNormalizeArguments, "unit/OperatorTimesNormalizeArguments");
    ti_success!(ti_success_unit_Optional, "unit/Optional");
    // ti_success!(ti_success_unit_OptionalBuild, "unit/OptionalBuild");
    ti_success!(ti_success_unit_OptionalFold, "unit/OptionalFold");
    ti_success!(ti_success_unit_RecordEmpty, "unit/RecordEmpty");
    ti_success!(ti_success_unit_RecordOneKind, "unit/RecordOneKind");
    ti_success!(ti_success_unit_RecordOneType, "unit/RecordOneType");
    ti_success!(ti_success_unit_RecordOneValue, "unit/RecordOneValue");
    // ti_success!(ti_success_unit_RecordProjectionEmpty, "unit/RecordProjectionEmpty");
    // ti_success!(ti_success_unit_RecordProjectionKind, "unit/RecordProjectionKind");
    // ti_success!(ti_success_unit_RecordProjectionType, "unit/RecordProjectionType");
    // ti_success!(ti_success_unit_RecordProjectionValue, "unit/RecordProjectionValue");
    // ti_success!(ti_success_unit_RecordSelectionKind, "unit/RecordSelectionKind");
    // ti_success!(ti_success_unit_RecordSelectionType, "unit/RecordSelectionType");
    ti_success!(ti_success_unit_RecordSelectionValue, "unit/RecordSelectionValue");
    ti_success!(ti_success_unit_RecordType, "unit/RecordType");
    ti_success!(ti_success_unit_RecordTypeEmpty, "unit/RecordTypeEmpty");
    ti_success!(ti_success_unit_RecordTypeKind, "unit/RecordTypeKind");
    ti_success!(ti_success_unit_RecordTypeType, "unit/RecordTypeType");
    // ti_success!(ti_success_unit_RecursiveRecordMergeLhsEmpty, "unit/RecursiveRecordMergeLhsEmpty");
    // ti_success!(ti_success_unit_RecursiveRecordMergeRecursively, "unit/RecursiveRecordMergeRecursively");
    // ti_success!(ti_success_unit_RecursiveRecordMergeRecursivelyTypes, "unit/RecursiveRecordMergeRecursivelyTypes");
    // ti_success!(ti_success_unit_RecursiveRecordMergeRhsEmpty, "unit/RecursiveRecordMergeRhsEmpty");
    // ti_success!(ti_success_unit_RecursiveRecordMergeTwo, "unit/RecursiveRecordMergeTwo");
    // ti_success!(ti_success_unit_RecursiveRecordMergeTwoKinds, "unit/RecursiveRecordMergeTwoKinds");
    // ti_success!(ti_success_unit_RecursiveRecordMergeTwoTypes, "unit/RecursiveRecordMergeTwoTypes");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeRecursively, "unit/RecursiveRecordTypeMergeRecursively");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeRecursivelyTypes, "unit/RecursiveRecordTypeMergeRecursivelyTypes");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeRhsEmpty, "unit/RecursiveRecordTypeMergeRhsEmpty");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeTwo, "unit/RecursiveRecordTypeMergeTwo");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeTwoKinds, "unit/RecursiveRecordTypeMergeTwoKinds");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeTwoTypes, "unit/RecursiveRecordTypeMergeTwoTypes");
    // ti_success!(ti_success_unit_RightBiasedRecordMergeRhsEmpty, "unit/RightBiasedRecordMergeRhsEmpty");
    // ti_success!(ti_success_unit_RightBiasedRecordMergeTwo, "unit/RightBiasedRecordMergeTwo");
    // ti_success!(ti_success_unit_RightBiasedRecordMergeTwoDifferent, "unit/RightBiasedRecordMergeTwoDifferent");
    // ti_success!(ti_success_unit_RightBiasedRecordMergeTwoKinds, "unit/RightBiasedRecordMergeTwoKinds");
    // ti_success!(ti_success_unit_RightBiasedRecordMergeTwoTypes, "unit/RightBiasedRecordMergeTwoTypes");
    ti_success!(ti_success_unit_SomeTrue, "unit/SomeTrue");
    ti_success!(ti_success_unit_Text, "unit/Text");
    ti_success!(ti_success_unit_TextLiteral, "unit/TextLiteral");
    ti_success!(ti_success_unit_TextLiteralNormalizeArguments, "unit/TextLiteralNormalizeArguments");
    ti_success!(ti_success_unit_TextLiteralWithInterpolation, "unit/TextLiteralWithInterpolation");
    // ti_success!(ti_success_unit_TextShow, "unit/TextShow");
    ti_success!(ti_success_unit_True, "unit/True");
    ti_success!(ti_success_unit_Type, "unit/Type");
    ti_success!(ti_success_unit_TypeAnnotation, "unit/TypeAnnotation");
    ti_success!(ti_success_unit_UnionConstructorField, "unit/UnionConstructorField");
    ti_success!(ti_success_unit_UnionOne, "unit/UnionOne");
    ti_success!(ti_success_unit_UnionTypeEmpty, "unit/UnionTypeEmpty");
    ti_success!(ti_success_unit_UnionTypeKind, "unit/UnionTypeKind");
    ti_success!(ti_success_unit_UnionTypeOne, "unit/UnionTypeOne");
    ti_success!(ti_success_unit_UnionTypeType, "unit/UnionTypeType");
}
