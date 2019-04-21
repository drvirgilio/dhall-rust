#![allow(non_snake_case)]
use std::collections::BTreeMap;
use std::rc::Rc;

use dhall_core::context::Context;
use dhall_core::{
    rc, shift, shift0, Builtin, ExprF, Integer, InterpolatedText,
    InterpolatedTextContents, Label, Natural, SubExpr, V, X,
};
use dhall_generator as dhall;

use crate::expr::{Normalized, Typed};

type InputSubExpr = SubExpr<X, Normalized<'static>>;
type OutputSubExpr = SubExpr<X, X>;

impl<'a> Typed<'a> {
    pub fn normalize(self) -> Normalized<'a> {
        Normalized(normalize(self.0), self.1, self.2)
    }
    pub fn normalize_ctx(
        self,
        ctx: &crate::typecheck::TypecheckContext,
    ) -> Normalized<'a> {
        Normalized(
            normalize_whnf(
                NormalizationContext::from_typecheck_ctx(ctx),
                self.0,
            )
            .normalize_to_expr(),
            self.1,
            self.2,
        )
    }
    /// Pretends this expression is normalized. Use with care.
    #[allow(dead_code)]
    pub fn skip_normalize(self) -> Normalized<'a> {
        Normalized(
            self.0.unroll().squash_embed(|e| e.0.clone()),
            self.1,
            self.2,
        )
    }
}

fn shift0_mut<N, E>(delta: isize, label: &Label, in_expr: &mut SubExpr<N, E>) {
    let new_expr = shift0(delta, label, &in_expr);
    std::mem::replace(in_expr, new_expr);
}

fn shift_mut<N, E>(delta: isize, var: &V<Label>, in_expr: &mut SubExpr<N, E>) {
    let new_expr = shift(delta, var, &in_expr);
    std::mem::replace(in_expr, new_expr);
}

fn apply_builtin(b: Builtin, args: Vec<WHNF>) -> WHNF {
    use dhall_core::Builtin::*;
    use WHNF::*;

    let ret = match b {
        OptionalNone => improved_slice_patterns::match_vec!(args;
            [t] => EmptyOptionalLit(Now::from_whnf(t)),
        ),
        NaturalIsZero => improved_slice_patterns::match_vec!(args;
            [NaturalLit(n)] => BoolLit(n == 0),
        ),
        NaturalEven => improved_slice_patterns::match_vec!(args;
            [NaturalLit(n)] => BoolLit(n % 2 == 0),
        ),
        NaturalOdd => improved_slice_patterns::match_vec!(args;
            [NaturalLit(n)] => BoolLit(n % 2 != 0),
        ),
        NaturalToInteger => improved_slice_patterns::match_vec!(args;
            [NaturalLit(n)] => IntegerLit(n as isize),
        ),
        NaturalShow => improved_slice_patterns::match_vec!(args;
            [NaturalLit(n)] => {
                TextLit(vec![InterpolatedTextContents::Text(n.to_string())])
            }
        ),
        ListLength => improved_slice_patterns::match_vec!(args;
           [_, EmptyListLit(_)] => NaturalLit(0),
           [_, NEListLit(xs)] => NaturalLit(xs.len()),
        ),
        ListHead => improved_slice_patterns::match_vec!(args;
            [_, EmptyListLit(n)] => EmptyOptionalLit(n),
            [_, NEListLit(xs)] => {
                NEOptionalLit(xs.into_iter().next().unwrap())
            }
        ),
        ListLast => improved_slice_patterns::match_vec!(args;
            [_, EmptyListLit(n)] => EmptyOptionalLit(n),
            [_, NEListLit(xs)] => {
                NEOptionalLit(xs.into_iter().rev().next().unwrap())
            }
        ),
        ListReverse => improved_slice_patterns::match_vec!(args;
            [_, EmptyListLit(n)] => EmptyListLit(n),
            [_, NEListLit(xs)] => {
                let mut xs = xs;
                xs.reverse();
                NEListLit(xs)
            }
        ),
        ListIndexed => improved_slice_patterns::match_vec!(args;
            [_, EmptyListLit(t)] => {
                let mut kts = BTreeMap::new();
                kts.insert(
                    "index".into(),
                    Now::from_whnf(
                        WHNF::from_builtin(Natural)
                    ),
                );
                kts.insert("value".into(), t);
                EmptyListLit(Now::from_whnf(RecordType(kts)))
            },
            [_, NEListLit(xs)] => {
                let xs = xs
                    .into_iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let i = NaturalLit(i);
                        let mut kvs = BTreeMap::new();
                        kvs.insert("index".into(), Now::from_whnf(i));
                        kvs.insert("value".into(), e);
                        Now::from_whnf(RecordLit(kvs))
                    })
                    .collect();
                NEListLit(xs)
            }
        ),
        ListBuild => improved_slice_patterns::match_vec!(args;
            // fold/build fusion
            [_, WHNF::AppliedBuiltin(ListFold, args)] => {
                let mut args = args;
                if args.len() >= 2 {
                    args.remove(1)
                } else {
                    // Do we really need to handle this case ?
                    unimplemented!()
                }
            },
            [t, g] => g
                .app(WHNF::from_builtin(List).app(t.clone()))
                .app(ListConsClosure(Now::from_whnf(t.clone()), None))
                .app(EmptyListLit(Now::from_whnf(t))),
        ),
        ListFold => improved_slice_patterns::match_vec!(args;
            // fold/build fusion
            [_, WHNF::AppliedBuiltin(ListBuild, args)] => {
                let mut args = args;
                if args.len() >= 2 {
                    args.remove(1)
                } else {
                    unimplemented!()
                }
            },
            [_, EmptyListLit(_), _, _, nil] => nil,
            [_, NEListLit(xs), _, cons, nil] => {
                let mut v = nil;
                for x in xs.into_iter().rev() {
                    v = cons.clone().app(x.normalize()).app(v);
                }
                v
            }
        ),
        OptionalBuild => improved_slice_patterns::match_vec!(args;
            // fold/build fusion
            [_, WHNF::AppliedBuiltin(OptionalFold, args)] => {
                let mut args = args;
                if args.len() >= 2 {
                    args.remove(1)
                } else {
                    unimplemented!()
                }
            },
            [t, g] => g
                .app(WHNF::from_builtin(Optional).app(t.clone()))
                .app(OptionalSomeClosure(Now::from_whnf(t.clone())))
                .app(EmptyOptionalLit(Now::from_whnf(t))),
        ),
        OptionalFold => improved_slice_patterns::match_vec!(args;
            // fold/build fusion
            [_, WHNF::AppliedBuiltin(OptionalBuild, args)] => {
                let mut args = args;
                if args.len() >= 2 {
                    args.remove(1)
                } else {
                    unimplemented!()
                }
            },
            [_, EmptyOptionalLit(_), _, _, nothing] => {
                nothing
            },
            [_, NEOptionalLit(x), _, just, _] => {
                just.app(x.normalize())
            }
        ),
        NaturalBuild => improved_slice_patterns::match_vec!(args;
            // fold/build fusion
            [WHNF::AppliedBuiltin(NaturalFold, args)] => {
                let mut args = args;
                if args.len() >= 1 {
                    args.remove(0)
                } else {
                    unimplemented!()
                }
            },
            [g] => g
                .app(WHNF::from_builtin(Natural))
                .app(NaturalSuccClosure)
                .app(NaturalLit(0)),
        ),
        NaturalFold => improved_slice_patterns::match_vec!(args;
            // fold/build fusion
            [WHNF::AppliedBuiltin(NaturalBuild, args)] => {
                let mut args = args;
                if args.len() >= 1 {
                    args.remove(0)
                } else {
                    unimplemented!()
                }
            },
            [NaturalLit(0), _, _, zero] => zero,
            [NaturalLit(n), t, succ, zero] => {
                let fold = WHNF::from_builtin(NaturalFold)
                    .app(NaturalLit(n - 1))
                    .app(t)
                    .app(succ.clone())
                    .app(zero);
                succ.app(fold)
            },
        ),
        _ => Err(args),
    };

    match ret {
        Ok(x) => x,
        Err(args) => AppliedBuiltin(b, args),
    }
}

#[derive(Debug, Clone)]
enum EnvItem {
    Expr(WHNF),
    Skip(V<Label>),
}

impl EnvItem {
    fn shift0(&self, delta: isize, x: &Label) -> Self {
        use EnvItem::*;
        match self {
            Expr(e) => {
                let mut e = e.clone();
                e.shift0(delta, x);
                Expr(e)
            }
            Skip(var) => Skip(var.shift0(delta, x)),
        }
    }
}

#[derive(Debug, Clone)]
struct NormalizationContext(Rc<Context<Label, EnvItem>>);

impl NormalizationContext {
    fn new() -> Self {
        NormalizationContext(Rc::new(Context::new()))
    }
    fn insert(&self, x: &Label, e: WHNF) -> Self {
        NormalizationContext(Rc::new(
            self.0.insert(x.clone(), EnvItem::Expr(e)),
        ))
    }
    fn skip(&self, x: &Label) -> Self {
        NormalizationContext(Rc::new(
            self.0
                .map(|_, e| e.shift0(1, x))
                .insert(x.clone(), EnvItem::Skip(V(x.clone(), 0))),
        ))
    }
    fn lookup(&self, var: &V<Label>) -> WHNF {
        let V(x, n) = var;
        match self.0.lookup(x, *n) {
            Some(EnvItem::Expr(e)) => e.clone(),
            Some(EnvItem::Skip(newvar)) => {
                WHNF::Expr(rc(ExprF::Var(newvar.clone())))
            }
            None => WHNF::Expr(rc(ExprF::Var(var.clone()))),
        }
    }
    fn from_typecheck_ctx(tc_ctx: &crate::typecheck::TypecheckContext) -> Self {
        use crate::typecheck::EnvItem::*;
        let mut ctx = Context::new();
        for (k, vs) in tc_ctx.0.iter_keys() {
            for v in vs.iter() {
                let new_item = match v {
                    Type(var, _) => EnvItem::Skip(var.clone()),
                    Value(e) => EnvItem::Expr(normalize_whnf(
                        NormalizationContext::new(),
                        e.as_expr().embed_absurd(),
                    )),
                };
                ctx = ctx.insert(k.clone(), new_item);
            }
        }
        NormalizationContext(Rc::new(ctx))
    }
}

/// A semantic value. This is partially redundant with `dhall_core::Expr`, on purpose. `Expr` should
/// be limited to syntactic expressions: either written by the user or meant to be printed.
/// The rule is the following: we must _not_ construct values of type `Expr` while normalizing,
/// but only construct `WHNF`s.
///
/// WHNFs store subexpressions unnormalized, to enable lazy normalization. They approximate
/// what's called Weak Head Normal-Form (WHNF). This means that the expression is normalized as
/// little as possible, but just enough to know the first constructor of the normal form. This is
/// identical to full normalization for simple types like integers, but for e.g. a record literal
/// this means knowing just the field names.
///
/// Each variant captures the relevant contexts when it is constructed. Conceptually each
/// subexpression has its own context, but in practice some contexts can be shared when sensible, to
/// avoid unnecessary allocations.
#[derive(Debug, Clone)]
enum WHNF {
    /// Closures
    Lam(Label, Now, (NormalizationContext, InputSubExpr)),
    AppliedBuiltin(Builtin, Vec<WHNF>),
    /// `λ(x: a) -> Some x`
    OptionalSomeClosure(Now),
    /// `λ(x : a) -> λ(xs : List a) -> [ x ] # xs`
    /// `λ(xs : List a) -> [ x ] # xs`
    ListConsClosure(Now, Option<Now>),
    /// `λ(x : Natural) -> x + 1`
    NaturalSuccClosure,

    BoolLit(bool),
    NaturalLit(Natural),
    IntegerLit(Integer),
    EmptyOptionalLit(Now),
    NEOptionalLit(Now),
    EmptyListLit(Now),
    NEListLit(Vec<Now>),
    RecordLit(BTreeMap<Label, Now>),
    RecordType(BTreeMap<Label, Now>),
    UnionType(NormalizationContext, BTreeMap<Label, Option<InputSubExpr>>),
    UnionConstructor(
        NormalizationContext,
        Label,
        BTreeMap<Label, Option<InputSubExpr>>,
    ),
    UnionLit(
        Label,
        Now,
        (NormalizationContext, BTreeMap<Label, Option<InputSubExpr>>),
    ),
    TextLit(Vec<InterpolatedTextContents<Now>>),
    /// This must not contain a value captured by one of the variants above.
    Expr(OutputSubExpr),
}

impl WHNF {
    /// Convert the value back to a (normalized) syntactic expression
    fn normalize_to_expr(self) -> OutputSubExpr {
        match self {
            WHNF::Lam(x, t, (ctx, e)) => {
                let ctx2 = ctx.skip(&x);
                rc(ExprF::Lam(
                    x.clone(),
                    t.normalize().normalize_to_expr(),
                    normalize_whnf(ctx2, e).normalize_to_expr(),
                ))
            }
            WHNF::AppliedBuiltin(b, args) => {
                let mut e = WHNF::Expr(rc(ExprF::Builtin(b)));
                for v in args {
                    e = e.app(v);
                }
                e.normalize_to_expr()
            }
            WHNF::OptionalSomeClosure(n) => {
                let a = n.normalize().normalize_to_expr();
                dhall::subexpr!(λ(x: a) -> Some x)
            }
            WHNF::ListConsClosure(n, None) => {
                let a = n.normalize().normalize_to_expr();
                // Avoid accidental capture of the new `x` variable
                let a1 = shift0(1, &"x".into(), &a);
                dhall::subexpr!(λ(x : a) -> λ(xs : List a1) -> [ x ] # xs)
            }
            WHNF::ListConsClosure(n, Some(v)) => {
                let v = v.normalize().normalize_to_expr();
                let a = n.normalize().normalize_to_expr();
                dhall::subexpr!(λ(xs : List a) -> [ v ] # xs)
            }
            WHNF::NaturalSuccClosure => {
                dhall::subexpr!(λ(x : Natural) -> x + 1)
            }
            WHNF::BoolLit(b) => rc(ExprF::BoolLit(b)),
            WHNF::NaturalLit(n) => rc(ExprF::NaturalLit(n)),
            WHNF::IntegerLit(n) => rc(ExprF::IntegerLit(n)),
            WHNF::EmptyOptionalLit(n) => {
                WHNF::Expr(rc(ExprF::Builtin(Builtin::OptionalNone)))
                    .app(n.normalize())
                    .normalize_to_expr()
            }
            WHNF::NEOptionalLit(n) => {
                rc(ExprF::SomeLit(n.normalize().normalize_to_expr()))
            }
            WHNF::EmptyListLit(n) => {
                rc(ExprF::EmptyListLit(n.normalize().normalize_to_expr()))
            }
            WHNF::NEListLit(elts) => rc(ExprF::NEListLit(
                elts.into_iter()
                    .map(|n| n.normalize().normalize_to_expr())
                    .collect(),
            )),
            WHNF::RecordLit(kvs) => rc(ExprF::RecordLit(
                kvs.into_iter()
                    .map(|(k, v)| (k, v.normalize().normalize_to_expr()))
                    .collect(),
            )),
            WHNF::RecordType(kts) => rc(ExprF::RecordType(
                kts.into_iter()
                    .map(|(k, v)| (k, v.normalize().normalize_to_expr()))
                    .collect(),
            )),
            WHNF::UnionType(ctx, kts) => rc(ExprF::UnionType(
                kts.into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            v.map(|v| {
                                normalize_whnf(ctx.clone(), v)
                                    .normalize_to_expr()
                            }),
                        )
                    })
                    .collect(),
            )),
            WHNF::UnionConstructor(ctx, l, kts) => {
                let kts = kts
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            v.map(|v| {
                                normalize_whnf(ctx.clone(), v)
                                    .normalize_to_expr()
                            }),
                        )
                    })
                    .collect();
                rc(ExprF::Field(rc(ExprF::UnionType(kts)), l))
            }
            WHNF::UnionLit(l, v, (kts_ctx, kts)) => rc(ExprF::UnionLit(
                l,
                v.normalize().normalize_to_expr(),
                kts.into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            v.map(|v| {
                                normalize_whnf(kts_ctx.clone(), v)
                                    .normalize_to_expr()
                            }),
                        )
                    })
                    .collect(),
            )),
            WHNF::TextLit(elts) => {
                fn normalize_textlit(
                    elts: Vec<InterpolatedTextContents<Now>>,
                ) -> InterpolatedText<OutputSubExpr> {
                    elts.into_iter()
                        .flat_map(|contents| {
                            use InterpolatedTextContents::{Expr, Text};
                            let new_interpolated = match contents {
                                Expr(n) => match n.normalize() {
                                    WHNF::TextLit(elts2) => {
                                        normalize_textlit(elts2)
                                    }
                                    e => InterpolatedText::from((
                                        String::new(),
                                        vec![(
                                            e.normalize_to_expr(),
                                            String::new(),
                                        )],
                                    )),
                                },
                                Text(s) => InterpolatedText::from(s),
                            };
                            new_interpolated.into_iter()
                        })
                        .collect()
                }

                rc(ExprF::TextLit(normalize_textlit(elts)))
            }
            WHNF::Expr(e) => e,
        }
    }

    /// Apply to a value
    fn app(self, val: WHNF) -> WHNF {
        match (self, val) {
            (WHNF::Lam(x, _, (ctx, e)), val) => {
                let ctx2 = ctx.insert(&x, val);
                normalize_whnf(ctx2, e)
            }
            (WHNF::AppliedBuiltin(b, mut args), val) => {
                args.push(val);
                apply_builtin(b, args)
            }
            (WHNF::OptionalSomeClosure(_), val) => {
                WHNF::NEOptionalLit(Now::from_whnf(val))
            }
            (WHNF::ListConsClosure(t, None), val) => {
                WHNF::ListConsClosure(t, Some(Now::from_whnf(val)))
            }
            (WHNF::ListConsClosure(_, Some(x)), WHNF::EmptyListLit(_)) => {
                WHNF::NEListLit(vec![x])
            }
            (WHNF::ListConsClosure(_, Some(x)), WHNF::NEListLit(mut xs)) => {
                xs.insert(0, x);
                WHNF::NEListLit(xs)
            }
            (WHNF::NaturalSuccClosure, WHNF::NaturalLit(n)) => {
                WHNF::NaturalLit(n + 1)
            }
            (WHNF::UnionConstructor(ctx, l, kts), val) => {
                WHNF::UnionLit(l, Now::from_whnf(val), (ctx, kts))
            }
            // Can't do anything useful, convert to expr
            (f, a) => WHNF::Expr(rc(ExprF::App(
                f.normalize_to_expr(),
                a.normalize_to_expr(),
            ))),
        }
    }

    fn from_builtin(b: Builtin) -> WHNF {
        WHNF::AppliedBuiltin(b, vec![])
    }

    fn shift0(&mut self, delta: isize, label: &Label) {
        match self {
            WHNF::NaturalSuccClosure
            | WHNF::BoolLit(_)
            | WHNF::NaturalLit(_)
            | WHNF::IntegerLit(_) => {}
            WHNF::EmptyOptionalLit(n)
            | WHNF::NEOptionalLit(n)
            | WHNF::OptionalSomeClosure(n)
            | WHNF::EmptyListLit(n) => {
                n.shift0(delta, label);
            }
            WHNF::Lam(_, t, (_, e)) => {
                t.shift0(delta, label);
                shift_mut(delta, &V(label.clone(), 1), e);
            }
            WHNF::AppliedBuiltin(_, args) => {
                for x in args.iter_mut() {
                    x.shift0(delta, label);
                }
            }
            WHNF::ListConsClosure(t, v) => {
                t.shift0(delta, label);
                for x in v.iter_mut() {
                    x.shift0(delta, label);
                }
            }
            WHNF::NEListLit(elts) => {
                for x in elts.iter_mut() {
                    x.shift0(delta, label);
                }
            }
            WHNF::RecordLit(kvs) | WHNF::RecordType(kvs) => {
                for x in kvs.values_mut() {
                    x.shift0(delta, label);
                }
            }
            WHNF::UnionType(_, kts) | WHNF::UnionConstructor(_, _, kts) => {
                for x in kts.values_mut().flat_map(|opt| opt) {
                    shift0_mut(delta, label, x);
                }
            }
            WHNF::UnionLit(_, v, (_, kts)) => {
                v.shift0(delta, label);
                for x in kts.values_mut().flat_map(|opt| opt) {
                    shift0_mut(delta, label, x);
                }
            }
            WHNF::TextLit(elts) => {
                for x in elts.iter_mut() {
                    use InterpolatedTextContents::{Expr, Text};
                    match x {
                        Expr(n) => n.shift0(delta, label),
                        Text(_) => {}
                    }
                }
            }
            WHNF::Expr(e) => shift0_mut(delta, label, e),
        }
    }
}

/// Normalize-on-write smart container. Contains either a (partially) normalized value or a
/// non-normalized value alongside a normalization context.
/// The name is a pun on std::borrow::Cow.
#[derive(Debug, Clone)]
enum Now {
    Normalized(Box<WHNF>),
    Unnormalized(NormalizationContext, InputSubExpr),
}

impl Now {
    fn new(ctx: NormalizationContext, e: InputSubExpr) -> Now {
        Now::Unnormalized(ctx, e)
    }

    fn from_whnf(v: WHNF) -> Now {
        Now::Normalized(Box::new(v))
    }

    fn normalize(self) -> WHNF {
        match self {
            Now::Normalized(v) => *v,
            Now::Unnormalized(ctx, e) => normalize_whnf(ctx, e),
        }
    }

    fn shift0(&mut self, delta: isize, label: &Label) {
        match self {
            Now::Normalized(v) => v.shift0(delta, label),
            Now::Unnormalized(_, e) => shift0_mut(delta, label, e),
        }
    }
}

/// Reduces the imput expression to WHNF. See doc on `WHNF` for details.
fn normalize_whnf(ctx: NormalizationContext, expr: InputSubExpr) -> WHNF {
    match expr.as_ref() {
        ExprF::Var(v) => ctx.lookup(&v),
        ExprF::Annot(x, _) => normalize_whnf(ctx, x.clone()),
        ExprF::Note(_, e) => normalize_whnf(ctx, e.clone()),
        // TODO: wasteful to retraverse everything
        ExprF::Embed(e) => normalize_whnf(ctx, e.0.embed_absurd()),
        ExprF::Let(x, _, r, b) => {
            let r = normalize_whnf(ctx.clone(), r.clone());
            normalize_whnf(ctx.insert(x, r), b.clone())
        }
        ExprF::Lam(x, t, e) => WHNF::Lam(
            x.clone(),
            Now::new(ctx.clone(), t.clone()),
            (ctx, e.clone()),
        ),
        ExprF::Builtin(b) => WHNF::AppliedBuiltin(*b, vec![]),
        ExprF::BoolLit(b) => WHNF::BoolLit(*b),
        ExprF::NaturalLit(n) => WHNF::NaturalLit(*n),
        ExprF::OldOptionalLit(None, e) => {
            WHNF::EmptyOptionalLit(Now::new(ctx, e.clone()))
        }
        ExprF::OldOptionalLit(Some(e), _) => {
            WHNF::NEOptionalLit(Now::new(ctx, e.clone()))
        }
        ExprF::SomeLit(e) => WHNF::NEOptionalLit(Now::new(ctx, e.clone())),
        ExprF::EmptyListLit(e) => WHNF::EmptyListLit(Now::new(ctx, e.clone())),
        ExprF::NEListLit(elts) => WHNF::NEListLit(
            elts.iter()
                .map(|e| Now::new(ctx.clone(), e.clone()))
                .collect(),
        ),
        ExprF::RecordLit(kvs) => WHNF::RecordLit(
            kvs.iter()
                .map(|(k, e)| (k.clone(), Now::new(ctx.clone(), e.clone())))
                .collect(),
        ),
        ExprF::UnionType(kvs) => WHNF::UnionType(ctx, kvs.clone()),
        ExprF::UnionLit(l, x, kts) => WHNF::UnionLit(
            l.clone(),
            Now::new(ctx.clone(), x.clone()),
            (ctx, kts.clone()),
        ),
        ExprF::TextLit(elts) => WHNF::TextLit(
            elts.iter()
                .map(|contents| {
                    use InterpolatedTextContents::{Expr, Text};
                    match contents {
                        Expr(n) => Expr(Now::new(ctx.clone(), n.clone())),
                        Text(s) => Text(s.clone()),
                    }
                })
                .collect(),
        ),
        ExprF::BoolIf(b, e1, e2) => {
            let b = normalize_whnf(ctx.clone(), b.clone());
            match b {
                WHNF::BoolLit(true) => normalize_whnf(ctx, e1.clone()),
                WHNF::BoolLit(false) => normalize_whnf(ctx, e2.clone()),
                b => {
                    let e1 = normalize_whnf(ctx.clone(), e1.clone());
                    let e2 = normalize_whnf(ctx, e2.clone());
                    match (e1, e2) {
                        (WHNF::BoolLit(true), WHNF::BoolLit(false)) => b,
                        (e1, e2) => {
                            normalize_last_layer(ExprF::BoolIf(b, e1, e2))
                        }
                    }
                }
            }
        }
        expr => {
            // Recursively normalize subexpressions
            let expr: ExprF<WHNF, Label, X, X> = expr
                .map_ref_with_special_handling_of_binders(
                    |e| normalize_whnf(ctx.clone(), e.clone()),
                    |x, e| normalize_whnf(ctx.skip(x), e.clone()),
                    X::clone,
                    |_| unreachable!(),
                    Label::clone,
                );

            normalize_last_layer(expr)
        }
    }
}

/// When all sub-expressions have been (partially) normalized, eval the remaining toplevel layer.
fn normalize_last_layer(expr: ExprF<WHNF, Label, X, X>) -> WHNF {
    use dhall_core::BinOp::*;
    use WHNF::*;

    match expr {
        ExprF::App(v, a) => v.app(a),

        ExprF::BinOp(BoolAnd, BoolLit(true), y) => y,
        ExprF::BinOp(BoolAnd, x, BoolLit(true)) => x,
        ExprF::BinOp(BoolAnd, BoolLit(false), _) => BoolLit(false),
        ExprF::BinOp(BoolAnd, _, BoolLit(false)) => BoolLit(false),
        ExprF::BinOp(BoolOr, BoolLit(true), _) => BoolLit(true),
        ExprF::BinOp(BoolOr, _, BoolLit(true)) => BoolLit(true),
        ExprF::BinOp(BoolOr, BoolLit(false), y) => y,
        ExprF::BinOp(BoolOr, x, BoolLit(false)) => x,
        ExprF::BinOp(BoolEQ, BoolLit(true), y) => y,
        ExprF::BinOp(BoolEQ, x, BoolLit(true)) => x,
        ExprF::BinOp(BoolNE, BoolLit(false), y) => y,
        ExprF::BinOp(BoolNE, x, BoolLit(false)) => x,
        ExprF::BinOp(BoolEQ, BoolLit(x), BoolLit(y)) => BoolLit(x == y),
        ExprF::BinOp(BoolNE, BoolLit(x), BoolLit(y)) => BoolLit(x != y),

        ExprF::BinOp(NaturalPlus, NaturalLit(0), y) => y,
        ExprF::BinOp(NaturalPlus, x, NaturalLit(0)) => x,
        ExprF::BinOp(NaturalTimes, NaturalLit(0), _) => NaturalLit(0),
        ExprF::BinOp(NaturalTimes, _, NaturalLit(0)) => NaturalLit(0),
        ExprF::BinOp(NaturalTimes, NaturalLit(1), y) => y,
        ExprF::BinOp(NaturalTimes, x, NaturalLit(1)) => x,
        ExprF::BinOp(NaturalPlus, NaturalLit(x), NaturalLit(y)) => {
            NaturalLit(x + y)
        }
        ExprF::BinOp(NaturalTimes, NaturalLit(x), NaturalLit(y)) => {
            NaturalLit(x * y)
        }

        ExprF::BinOp(ListAppend, EmptyListLit(_), y) => y,
        ExprF::BinOp(ListAppend, x, EmptyListLit(_)) => x,
        ExprF::BinOp(ListAppend, NEListLit(mut xs), NEListLit(mut ys)) => {
            xs.append(&mut ys);
            NEListLit(xs)
        }
        ExprF::BinOp(TextAppend, TextLit(mut x), TextLit(mut y)) => {
            x.append(&mut y);
            TextLit(x)
        }
        ExprF::BinOp(TextAppend, TextLit(x), y) if x.is_empty() => y,
        ExprF::BinOp(TextAppend, x, TextLit(y)) if y.is_empty() => x,

        ExprF::Field(UnionType(ctx, kts), l) => UnionConstructor(ctx, l, kts),
        ExprF::Field(RecordLit(mut kvs), l) => match kvs.remove(&l) {
            Some(r) => r.normalize(),
            None => {
                // Couldn't do anything useful, so just normalize subexpressions
                Expr(rc(ExprF::Field(RecordLit(kvs).normalize_to_expr(), l)))
            }
        },
        ExprF::Projection(_, ls) if ls.is_empty() => {
            RecordLit(std::collections::BTreeMap::new())
        }
        ExprF::Projection(RecordLit(mut kvs), ls) => RecordLit(
            ls.into_iter()
                .filter_map(|l| kvs.remove(&l).map(|x| (l, x)))
                .collect(),
        ),
        ExprF::Merge(RecordLit(mut handlers), e, t) => {
            let e = match e {
                UnionConstructor(ctor_ctx, l, kts) => match handlers.remove(&l)
                {
                    Some(h) => return h.normalize(),
                    None => UnionConstructor(ctor_ctx, l, kts),
                },
                UnionLit(l, v, (kts_ctx, kts)) => match handlers.remove(&l) {
                    Some(h) => {
                        let h = h.normalize();
                        let v = v.normalize();
                        return h.app(v);
                    }
                    None => UnionLit(l, v, (kts_ctx, kts)),
                },
                e => e,
            };
            // Couldn't do anything useful, so just normalize subexpressions
            Expr(rc(ExprF::Merge(
                RecordLit(handlers).normalize_to_expr(),
                e.normalize_to_expr(),
                t.map(WHNF::normalize_to_expr),
            )))
        }
        // Couldn't do anything useful, so just normalize subexpressions
        expr => {
            Expr(rc(expr.map_ref_simple(|e| e.clone().normalize_to_expr())))
        }
    }
}

/// Reduce an expression to its normal form, performing beta reduction
///
/// `normalize` does not type-check the expression.  You may want to type-check
/// expressions before normalizing them since normalization can convert an
/// ill-typed expression into a well-typed expression.
///
/// However, `normalize` will not fail if the expression is ill-typed and will
/// leave ill-typed sub-expressions unevaluated.
///
fn normalize(e: InputSubExpr) -> OutputSubExpr {
    normalize_whnf(NormalizationContext::new(), e).normalize_to_expr()
}

#[cfg(test)]
mod spec_tests {
    #![rustfmt::skip]

    macro_rules! norm {
        ($name:ident, $path:expr) => {
            make_spec_test!(Normalization, Success, $name, $path);
        };
    }

    norm!(success_haskell_tutorial_access_0, "haskell-tutorial/access/0");
    norm!(success_haskell_tutorial_access_1, "haskell-tutorial/access/1");
    // norm!(success_haskell_tutorial_combineTypes_0, "haskell-tutorial/combineTypes/0");
    // norm!(success_haskell_tutorial_combineTypes_1, "haskell-tutorial/combineTypes/1");
    // norm!(success_haskell_tutorial_prefer_0, "haskell-tutorial/prefer/0");
    norm!(success_haskell_tutorial_projection_0, "haskell-tutorial/projection/0");


    norm!(success_prelude_Bool_and_0, "prelude/Bool/and/0");
    norm!(success_prelude_Bool_and_1, "prelude/Bool/and/1");
    norm!(success_prelude_Bool_build_0, "prelude/Bool/build/0");
    norm!(success_prelude_Bool_build_1, "prelude/Bool/build/1");
    norm!(success_prelude_Bool_even_0, "prelude/Bool/even/0");
    norm!(success_prelude_Bool_even_1, "prelude/Bool/even/1");
    norm!(success_prelude_Bool_even_2, "prelude/Bool/even/2");
    norm!(success_prelude_Bool_even_3, "prelude/Bool/even/3");
    norm!(success_prelude_Bool_fold_0, "prelude/Bool/fold/0");
    norm!(success_prelude_Bool_fold_1, "prelude/Bool/fold/1");
    norm!(success_prelude_Bool_not_0, "prelude/Bool/not/0");
    norm!(success_prelude_Bool_not_1, "prelude/Bool/not/1");
    norm!(success_prelude_Bool_odd_0, "prelude/Bool/odd/0");
    norm!(success_prelude_Bool_odd_1, "prelude/Bool/odd/1");
    norm!(success_prelude_Bool_odd_2, "prelude/Bool/odd/2");
    norm!(success_prelude_Bool_odd_3, "prelude/Bool/odd/3");
    norm!(success_prelude_Bool_or_0, "prelude/Bool/or/0");
    norm!(success_prelude_Bool_or_1, "prelude/Bool/or/1");
    norm!(success_prelude_Bool_show_0, "prelude/Bool/show/0");
    norm!(success_prelude_Bool_show_1, "prelude/Bool/show/1");
    // norm!(success_prelude_Double_show_0, "prelude/Double/show/0");
    // norm!(success_prelude_Double_show_1, "prelude/Double/show/1");
    // norm!(success_prelude_Integer_show_0, "prelude/Integer/show/0");
    // norm!(success_prelude_Integer_show_1, "prelude/Integer/show/1");
    // norm!(success_prelude_Integer_toDouble_0, "prelude/Integer/toDouble/0");
    // norm!(success_prelude_Integer_toDouble_1, "prelude/Integer/toDouble/1");
    norm!(success_prelude_List_all_0, "prelude/List/all/0");
    norm!(success_prelude_List_all_1, "prelude/List/all/1");
    norm!(success_prelude_List_any_0, "prelude/List/any/0");
    norm!(success_prelude_List_any_1, "prelude/List/any/1");
    norm!(success_prelude_List_build_0, "prelude/List/build/0");
    norm!(success_prelude_List_build_1, "prelude/List/build/1");
    norm!(success_prelude_List_concat_0, "prelude/List/concat/0");
    norm!(success_prelude_List_concat_1, "prelude/List/concat/1");
    norm!(success_prelude_List_concatMap_0, "prelude/List/concatMap/0");
    norm!(success_prelude_List_concatMap_1, "prelude/List/concatMap/1");
    norm!(success_prelude_List_filter_0, "prelude/List/filter/0");
    norm!(success_prelude_List_filter_1, "prelude/List/filter/1");
    norm!(success_prelude_List_fold_0, "prelude/List/fold/0");
    norm!(success_prelude_List_fold_1, "prelude/List/fold/1");
    norm!(success_prelude_List_fold_2, "prelude/List/fold/2");
    norm!(success_prelude_List_generate_0, "prelude/List/generate/0");
    norm!(success_prelude_List_generate_1, "prelude/List/generate/1");
    norm!(success_prelude_List_head_0, "prelude/List/head/0");
    norm!(success_prelude_List_head_1, "prelude/List/head/1");
    norm!(success_prelude_List_indexed_0, "prelude/List/indexed/0");
    norm!(success_prelude_List_indexed_1, "prelude/List/indexed/1");
    norm!(success_prelude_List_iterate_0, "prelude/List/iterate/0");
    norm!(success_prelude_List_iterate_1, "prelude/List/iterate/1");
    norm!(success_prelude_List_last_0, "prelude/List/last/0");
    norm!(success_prelude_List_last_1, "prelude/List/last/1");
    norm!(success_prelude_List_length_0, "prelude/List/length/0");
    norm!(success_prelude_List_length_1, "prelude/List/length/1");
    norm!(success_prelude_List_map_0, "prelude/List/map/0");
    norm!(success_prelude_List_map_1, "prelude/List/map/1");
    norm!(success_prelude_List_null_0, "prelude/List/null/0");
    norm!(success_prelude_List_null_1, "prelude/List/null/1");
    norm!(success_prelude_List_replicate_0, "prelude/List/replicate/0");
    norm!(success_prelude_List_replicate_1, "prelude/List/replicate/1");
    norm!(success_prelude_List_reverse_0, "prelude/List/reverse/0");
    norm!(success_prelude_List_reverse_1, "prelude/List/reverse/1");
    norm!(success_prelude_List_shifted_0, "prelude/List/shifted/0");
    norm!(success_prelude_List_shifted_1, "prelude/List/shifted/1");
    norm!(success_prelude_List_unzip_0, "prelude/List/unzip/0");
    norm!(success_prelude_List_unzip_1, "prelude/List/unzip/1");
    norm!(success_prelude_Natural_build_0, "prelude/Natural/build/0");
    norm!(success_prelude_Natural_build_1, "prelude/Natural/build/1");
    norm!(success_prelude_Natural_enumerate_0, "prelude/Natural/enumerate/0");
    norm!(success_prelude_Natural_enumerate_1, "prelude/Natural/enumerate/1");
    norm!(success_prelude_Natural_even_0, "prelude/Natural/even/0");
    norm!(success_prelude_Natural_even_1, "prelude/Natural/even/1");
    norm!(success_prelude_Natural_fold_0, "prelude/Natural/fold/0");
    norm!(success_prelude_Natural_fold_1, "prelude/Natural/fold/1");
    norm!(success_prelude_Natural_fold_2, "prelude/Natural/fold/2");
    norm!(success_prelude_Natural_isZero_0, "prelude/Natural/isZero/0");
    norm!(success_prelude_Natural_isZero_1, "prelude/Natural/isZero/1");
    norm!(success_prelude_Natural_odd_0, "prelude/Natural/odd/0");
    norm!(success_prelude_Natural_odd_1, "prelude/Natural/odd/1");
    norm!(success_prelude_Natural_product_0, "prelude/Natural/product/0");
    norm!(success_prelude_Natural_product_1, "prelude/Natural/product/1");
    // norm!(success_prelude_Natural_show_0, "prelude/Natural/show/0");
    // norm!(success_prelude_Natural_show_1, "prelude/Natural/show/1");
    norm!(success_prelude_Natural_sum_0, "prelude/Natural/sum/0");
    norm!(success_prelude_Natural_sum_1, "prelude/Natural/sum/1");
    // norm!(success_prelude_Natural_toDouble_0, "prelude/Natural/toDouble/0");
    // norm!(success_prelude_Natural_toDouble_1, "prelude/Natural/toDouble/1");
    // norm!(success_prelude_Natural_toInteger_0, "prelude/Natural/toInteger/0");
    // norm!(success_prelude_Natural_toInteger_1, "prelude/Natural/toInteger/1");
    norm!(success_prelude_Optional_all_0, "prelude/Optional/all/0");
    norm!(success_prelude_Optional_all_1, "prelude/Optional/all/1");
    norm!(success_prelude_Optional_any_0, "prelude/Optional/any/0");
    norm!(success_prelude_Optional_any_1, "prelude/Optional/any/1");
    // norm!(success_prelude_Optional_build_0, "prelude/Optional/build/0");
    // norm!(success_prelude_Optional_build_1, "prelude/Optional/build/1");
    norm!(success_prelude_Optional_concat_0, "prelude/Optional/concat/0");
    norm!(success_prelude_Optional_concat_1, "prelude/Optional/concat/1");
    norm!(success_prelude_Optional_concat_2, "prelude/Optional/concat/2");
    // norm!(success_prelude_Optional_filter_0, "prelude/Optional/filter/0");
    // norm!(success_prelude_Optional_filter_1, "prelude/Optional/filter/1");
    norm!(success_prelude_Optional_fold_0, "prelude/Optional/fold/0");
    norm!(success_prelude_Optional_fold_1, "prelude/Optional/fold/1");
    norm!(success_prelude_Optional_head_0, "prelude/Optional/head/0");
    norm!(success_prelude_Optional_head_1, "prelude/Optional/head/1");
    norm!(success_prelude_Optional_head_2, "prelude/Optional/head/2");
    norm!(success_prelude_Optional_last_0, "prelude/Optional/last/0");
    norm!(success_prelude_Optional_last_1, "prelude/Optional/last/1");
    norm!(success_prelude_Optional_last_2, "prelude/Optional/last/2");
    norm!(success_prelude_Optional_length_0, "prelude/Optional/length/0");
    norm!(success_prelude_Optional_length_1, "prelude/Optional/length/1");
    norm!(success_prelude_Optional_map_0, "prelude/Optional/map/0");
    norm!(success_prelude_Optional_map_1, "prelude/Optional/map/1");
    norm!(success_prelude_Optional_null_0, "prelude/Optional/null/0");
    norm!(success_prelude_Optional_null_1, "prelude/Optional/null/1");
    norm!(success_prelude_Optional_toList_0, "prelude/Optional/toList/0");
    norm!(success_prelude_Optional_toList_1, "prelude/Optional/toList/1");
    norm!(success_prelude_Optional_unzip_0, "prelude/Optional/unzip/0");
    norm!(success_prelude_Optional_unzip_1, "prelude/Optional/unzip/1");
    norm!(success_prelude_Text_concat_0, "prelude/Text/concat/0");
    norm!(success_prelude_Text_concat_1, "prelude/Text/concat/1");
    norm!(success_prelude_Text_concatMap_0, "prelude/Text/concatMap/0");
    norm!(success_prelude_Text_concatMap_1, "prelude/Text/concatMap/1");
    // norm!(success_prelude_Text_concatMapSep_0, "prelude/Text/concatMapSep/0");
    // norm!(success_prelude_Text_concatMapSep_1, "prelude/Text/concatMapSep/1");
    // norm!(success_prelude_Text_concatSep_0, "prelude/Text/concatSep/0");
    // norm!(success_prelude_Text_concatSep_1, "prelude/Text/concatSep/1");
    // norm!(success_prelude_Text_show_0, "prelude/Text/show/0");
    // norm!(success_prelude_Text_show_1, "prelude/Text/show/1");



    // norm!(success_remoteSystems, "remoteSystems");
    // norm!(success_simple_doubleShow, "simple/doubleShow");
    // norm!(success_simple_integerShow, "simple/integerShow");
    // norm!(success_simple_integerToDouble, "simple/integerToDouble");
    // norm!(success_simple_letlet, "simple/letlet");
    norm!(success_simple_listBuild, "simple/listBuild");
    norm!(success_simple_multiLine, "simple/multiLine");
    norm!(success_simple_naturalBuild, "simple/naturalBuild");
    norm!(success_simple_naturalPlus, "simple/naturalPlus");
    norm!(success_simple_naturalShow, "simple/naturalShow");
    norm!(success_simple_naturalToInteger, "simple/naturalToInteger");
    norm!(success_simple_optionalBuild, "simple/optionalBuild");
    norm!(success_simple_optionalBuildFold, "simple/optionalBuildFold");
    norm!(success_simple_optionalFold, "simple/optionalFold");
    // norm!(success_simple_sortOperator, "simple/sortOperator");
    // norm!(success_simplifications_and, "simplifications/and");
    // norm!(success_simplifications_eq, "simplifications/eq");
    // norm!(success_simplifications_ifThenElse, "simplifications/ifThenElse");
    // norm!(success_simplifications_ne, "simplifications/ne");
    // norm!(success_simplifications_or, "simplifications/or");


    norm!(success_unit_Bool, "unit/Bool");
    norm!(success_unit_Double, "unit/Double");
    norm!(success_unit_DoubleLiteral, "unit/DoubleLiteral");
    norm!(success_unit_DoubleShow, "unit/DoubleShow");
    // norm!(success_unit_DoubleShowValue, "unit/DoubleShowValue");
    norm!(success_unit_FunctionApplicationCapture, "unit/FunctionApplicationCapture");
    norm!(success_unit_FunctionApplicationNoSubstitute, "unit/FunctionApplicationNoSubstitute");
    norm!(success_unit_FunctionApplicationNormalizeArguments, "unit/FunctionApplicationNormalizeArguments");
    norm!(success_unit_FunctionApplicationSubstitute, "unit/FunctionApplicationSubstitute");
    norm!(success_unit_FunctionNormalizeArguments, "unit/FunctionNormalizeArguments");
    norm!(success_unit_FunctionTypeNormalizeArguments, "unit/FunctionTypeNormalizeArguments");
    // norm!(success_unit_IfAlternativesIdentical, "unit/IfAlternativesIdentical");
    norm!(success_unit_IfFalse, "unit/IfFalse");
    norm!(success_unit_IfNormalizePredicateAndBranches, "unit/IfNormalizePredicateAndBranches");
    norm!(success_unit_IfTrivial, "unit/IfTrivial");
    norm!(success_unit_IfTrue, "unit/IfTrue");
    norm!(success_unit_Integer, "unit/Integer");
    norm!(success_unit_IntegerNegative, "unit/IntegerNegative");
    norm!(success_unit_IntegerPositive, "unit/IntegerPositive");
    // norm!(success_unit_IntegerShow_12, "unit/IntegerShow-12");
    // norm!(success_unit_IntegerShow12, "unit/IntegerShow12");
    norm!(success_unit_IntegerShow, "unit/IntegerShow");
    // norm!(success_unit_IntegerToDouble_12, "unit/IntegerToDouble-12");
    // norm!(success_unit_IntegerToDouble12, "unit/IntegerToDouble12");
    norm!(success_unit_IntegerToDouble, "unit/IntegerToDouble");
    norm!(success_unit_Kind, "unit/Kind");
    norm!(success_unit_Let, "unit/Let");
    norm!(success_unit_LetWithType, "unit/LetWithType");
    norm!(success_unit_List, "unit/List");
    norm!(success_unit_ListBuild, "unit/ListBuild");
    norm!(success_unit_ListBuildFoldFusion, "unit/ListBuildFoldFusion");
    norm!(success_unit_ListBuildImplementation, "unit/ListBuildImplementation");
    norm!(success_unit_ListFold, "unit/ListFold");
    norm!(success_unit_ListFoldEmpty, "unit/ListFoldEmpty");
    norm!(success_unit_ListFoldOne, "unit/ListFoldOne");
    norm!(success_unit_ListHead, "unit/ListHead");
    norm!(success_unit_ListHeadEmpty, "unit/ListHeadEmpty");
    norm!(success_unit_ListHeadOne, "unit/ListHeadOne");
    norm!(success_unit_ListIndexed, "unit/ListIndexed");
    norm!(success_unit_ListIndexedEmpty, "unit/ListIndexedEmpty");
    norm!(success_unit_ListIndexedOne, "unit/ListIndexedOne");
    norm!(success_unit_ListLast, "unit/ListLast");
    norm!(success_unit_ListLastEmpty, "unit/ListLastEmpty");
    norm!(success_unit_ListLastOne, "unit/ListLastOne");
    norm!(success_unit_ListLength, "unit/ListLength");
    norm!(success_unit_ListLengthEmpty, "unit/ListLengthEmpty");
    norm!(success_unit_ListLengthOne, "unit/ListLengthOne");
    norm!(success_unit_ListNormalizeElements, "unit/ListNormalizeElements");
    norm!(success_unit_ListNormalizeTypeAnnotation, "unit/ListNormalizeTypeAnnotation");
    norm!(success_unit_ListReverse, "unit/ListReverse");
    norm!(success_unit_ListReverseEmpty, "unit/ListReverseEmpty");
    norm!(success_unit_ListReverseTwo, "unit/ListReverseTwo");
    norm!(success_unit_Merge, "unit/Merge");
    norm!(success_unit_MergeEmptyAlternative, "unit/MergeEmptyAlternative");
    norm!(success_unit_MergeNormalizeArguments, "unit/MergeNormalizeArguments");
    norm!(success_unit_MergeWithType, "unit/MergeWithType");
    norm!(success_unit_MergeWithTypeNormalizeArguments, "unit/MergeWithTypeNormalizeArguments");
    norm!(success_unit_Natural, "unit/Natural");
    norm!(success_unit_NaturalBuild, "unit/NaturalBuild");
    norm!(success_unit_NaturalBuildFoldFusion, "unit/NaturalBuildFoldFusion");
    norm!(success_unit_NaturalBuildImplementation, "unit/NaturalBuildImplementation");
    norm!(success_unit_NaturalEven, "unit/NaturalEven");
    norm!(success_unit_NaturalEvenOne, "unit/NaturalEvenOne");
    norm!(success_unit_NaturalEvenZero, "unit/NaturalEvenZero");
    norm!(success_unit_NaturalFold, "unit/NaturalFold");
    norm!(success_unit_NaturalFoldOne, "unit/NaturalFoldOne");
    norm!(success_unit_NaturalFoldZero, "unit/NaturalFoldZero");
    norm!(success_unit_NaturalIsZero, "unit/NaturalIsZero");
    norm!(success_unit_NaturalIsZeroOne, "unit/NaturalIsZeroOne");
    norm!(success_unit_NaturalIsZeroZero, "unit/NaturalIsZeroZero");
    norm!(success_unit_NaturalLiteral, "unit/NaturalLiteral");
    norm!(success_unit_NaturalOdd, "unit/NaturalOdd");
    norm!(success_unit_NaturalOddOne, "unit/NaturalOddOne");
    norm!(success_unit_NaturalOddZero, "unit/NaturalOddZero");
    norm!(success_unit_NaturalShow, "unit/NaturalShow");
    norm!(success_unit_NaturalShowOne, "unit/NaturalShowOne");
    norm!(success_unit_NaturalToInteger, "unit/NaturalToInteger");
    norm!(success_unit_NaturalToIntegerOne, "unit/NaturalToIntegerOne");
    norm!(success_unit_None, "unit/None");
    norm!(success_unit_NoneNatural, "unit/NoneNatural");
    // norm!(success_unit_OperatorAndEquivalentArguments, "unit/OperatorAndEquivalentArguments");
    norm!(success_unit_OperatorAndLhsFalse, "unit/OperatorAndLhsFalse");
    norm!(success_unit_OperatorAndLhsTrue, "unit/OperatorAndLhsTrue");
    norm!(success_unit_OperatorAndNormalizeArguments, "unit/OperatorAndNormalizeArguments");
    norm!(success_unit_OperatorAndRhsFalse, "unit/OperatorAndRhsFalse");
    norm!(success_unit_OperatorAndRhsTrue, "unit/OperatorAndRhsTrue");
    // norm!(success_unit_OperatorEqualEquivalentArguments, "unit/OperatorEqualEquivalentArguments");
    norm!(success_unit_OperatorEqualLhsTrue, "unit/OperatorEqualLhsTrue");
    norm!(success_unit_OperatorEqualNormalizeArguments, "unit/OperatorEqualNormalizeArguments");
    norm!(success_unit_OperatorEqualRhsTrue, "unit/OperatorEqualRhsTrue");
    norm!(success_unit_OperatorListConcatenateLhsEmpty, "unit/OperatorListConcatenateLhsEmpty");
    norm!(success_unit_OperatorListConcatenateListList, "unit/OperatorListConcatenateListList");
    norm!(success_unit_OperatorListConcatenateNormalizeArguments, "unit/OperatorListConcatenateNormalizeArguments");
    norm!(success_unit_OperatorListConcatenateRhsEmpty, "unit/OperatorListConcatenateRhsEmpty");
    // norm!(success_unit_OperatorNotEqualEquivalentArguments, "unit/OperatorNotEqualEquivalentArguments");
    norm!(success_unit_OperatorNotEqualLhsFalse, "unit/OperatorNotEqualLhsFalse");
    norm!(success_unit_OperatorNotEqualNormalizeArguments, "unit/OperatorNotEqualNormalizeArguments");
    norm!(success_unit_OperatorNotEqualRhsFalse, "unit/OperatorNotEqualRhsFalse");
    // norm!(success_unit_OperatorOrEquivalentArguments, "unit/OperatorOrEquivalentArguments");
    norm!(success_unit_OperatorOrLhsFalse, "unit/OperatorOrLhsFalse");
    norm!(success_unit_OperatorOrLhsTrue, "unit/OperatorOrLhsTrue");
    norm!(success_unit_OperatorOrNormalizeArguments, "unit/OperatorOrNormalizeArguments");
    norm!(success_unit_OperatorOrRhsFalse, "unit/OperatorOrRhsFalse");
    norm!(success_unit_OperatorOrRhsTrue, "unit/OperatorOrRhsTrue");
    norm!(success_unit_OperatorPlusLhsZero, "unit/OperatorPlusLhsZero");
    norm!(success_unit_OperatorPlusNormalizeArguments, "unit/OperatorPlusNormalizeArguments");
    norm!(success_unit_OperatorPlusOneAndOne, "unit/OperatorPlusOneAndOne");
    norm!(success_unit_OperatorPlusRhsZero, "unit/OperatorPlusRhsZero");
    norm!(success_unit_OperatorTextConcatenateLhsEmpty, "unit/OperatorTextConcatenateLhsEmpty");
    norm!(success_unit_OperatorTextConcatenateNormalizeArguments, "unit/OperatorTextConcatenateNormalizeArguments");
    norm!(success_unit_OperatorTextConcatenateRhsEmpty, "unit/OperatorTextConcatenateRhsEmpty");
    norm!(success_unit_OperatorTextConcatenateTextText, "unit/OperatorTextConcatenateTextText");
    norm!(success_unit_OperatorTimesLhsOne, "unit/OperatorTimesLhsOne");
    norm!(success_unit_OperatorTimesLhsZero, "unit/OperatorTimesLhsZero");
    norm!(success_unit_OperatorTimesNormalizeArguments, "unit/OperatorTimesNormalizeArguments");
    norm!(success_unit_OperatorTimesRhsOne, "unit/OperatorTimesRhsOne");
    norm!(success_unit_OperatorTimesRhsZero, "unit/OperatorTimesRhsZero");
    norm!(success_unit_OperatorTimesTwoAndTwo, "unit/OperatorTimesTwoAndTwo");
    norm!(success_unit_Optional, "unit/Optional");
    norm!(success_unit_OptionalBuild, "unit/OptionalBuild");
    norm!(success_unit_OptionalBuildFoldFusion, "unit/OptionalBuildFoldFusion");
    norm!(success_unit_OptionalBuildImplementation, "unit/OptionalBuildImplementation");
    norm!(success_unit_OptionalFold, "unit/OptionalFold");
    norm!(success_unit_OptionalFoldNone, "unit/OptionalFoldNone");
    norm!(success_unit_OptionalFoldSome, "unit/OptionalFoldSome");
    norm!(success_unit_Record, "unit/Record");
    norm!(success_unit_RecordEmpty, "unit/RecordEmpty");
    norm!(success_unit_RecordProjection, "unit/RecordProjection");
    norm!(success_unit_RecordProjectionEmpty, "unit/RecordProjectionEmpty");
    norm!(success_unit_RecordProjectionNormalizeArguments, "unit/RecordProjectionNormalizeArguments");
    norm!(success_unit_RecordSelection, "unit/RecordSelection");
    norm!(success_unit_RecordSelectionNormalizeArguments, "unit/RecordSelectionNormalizeArguments");
    norm!(success_unit_RecordType, "unit/RecordType");
    norm!(success_unit_RecordTypeEmpty, "unit/RecordTypeEmpty");
    // norm!(success_unit_RecursiveRecordMergeCollision, "unit/RecursiveRecordMergeCollision");
    // norm!(success_unit_RecursiveRecordMergeLhsEmpty, "unit/RecursiveRecordMergeLhsEmpty");
    // norm!(success_unit_RecursiveRecordMergeNoCollision, "unit/RecursiveRecordMergeNoCollision");
    // norm!(success_unit_RecursiveRecordMergeNormalizeArguments, "unit/RecursiveRecordMergeNormalizeArguments");
    // norm!(success_unit_RecursiveRecordMergeRhsEmpty, "unit/RecursiveRecordMergeRhsEmpty");
    // norm!(success_unit_RecursiveRecordTypeMergeCollision, "unit/RecursiveRecordTypeMergeCollision");
    // norm!(success_unit_RecursiveRecordTypeMergeLhsEmpty, "unit/RecursiveRecordTypeMergeLhsEmpty");
    // norm!(success_unit_RecursiveRecordTypeMergeNoCollision, "unit/RecursiveRecordTypeMergeNoCollision");
    // norm!(success_unit_RecursiveRecordTypeMergeNormalizeArguments, "unit/RecursiveRecordTypeMergeNormalizeArguments");
    // norm!(success_unit_RecursiveRecordTypeMergeRhsEmpty, "unit/RecursiveRecordTypeMergeRhsEmpty");
    // norm!(success_unit_RecursiveRecordTypeMergeSorts, "unit/RecursiveRecordTypeMergeSorts");
    // norm!(success_unit_RightBiasedRecordMergeCollision, "unit/RightBiasedRecordMergeCollision");
    // norm!(success_unit_RightBiasedRecordMergeLhsEmpty, "unit/RightBiasedRecordMergeLhsEmpty");
    // norm!(success_unit_RightBiasedRecordMergeNoCollision, "unit/RightBiasedRecordMergeNoCollision");
    // norm!(success_unit_RightBiasedRecordMergeNormalizeArguments, "unit/RightBiasedRecordMergeNormalizeArguments");
    // norm!(success_unit_RightBiasedRecordMergeRhsEmpty, "unit/RightBiasedRecordMergeRhsEmpty");
    norm!(success_unit_SomeNormalizeArguments, "unit/SomeNormalizeArguments");
    norm!(success_unit_Sort, "unit/Sort");
    norm!(success_unit_Text, "unit/Text");
    norm!(success_unit_TextInterpolate, "unit/TextInterpolate");
    norm!(success_unit_TextLiteral, "unit/TextLiteral");
    norm!(success_unit_TextNormalizeInterpolations, "unit/TextNormalizeInterpolations");
    norm!(success_unit_TextShow, "unit/TextShow");
    // norm!(success_unit_TextShowAllEscapes, "unit/TextShowAllEscapes");
    norm!(success_unit_True, "unit/True");
    norm!(success_unit_Type, "unit/Type");
    norm!(success_unit_TypeAnnotation, "unit/TypeAnnotation");
    norm!(success_unit_UnionNormalizeAlternatives, "unit/UnionNormalizeAlternatives");
    norm!(success_unit_UnionNormalizeArguments, "unit/UnionNormalizeArguments");
    norm!(success_unit_UnionProjectConstructor, "unit/UnionProjectConstructor");
    norm!(success_unit_UnionSortAlternatives, "unit/UnionSortAlternatives");
    norm!(success_unit_UnionType, "unit/UnionType");
    norm!(success_unit_UnionTypeEmpty, "unit/UnionTypeEmpty");
    norm!(success_unit_UnionTypeNormalizeArguments, "unit/UnionTypeNormalizeArguments");
    norm!(success_unit_Variable, "unit/Variable");
}
